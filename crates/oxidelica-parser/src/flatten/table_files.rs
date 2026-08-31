//! Tables written in a file: the text format the standard tables
//! read, and the MATLAB level 4 file they read beside it.
//!
//! A table block may be handed its numbers outright or told where to
//! find them. What is found is a matrix like any other, so the reading
//! ends where `tables` begins: a name, a file, and the rows they lead
//! to.

/// The rows of the table named in a file, or why they could not be
/// read.
///
/// The two formats are told apart by looking: a level 4 MATLAB file
/// starts with a header of five little-endian numbers whose first is
/// nearly always zero, and a text file starts with `#1`. Anything
/// else is refused by name rather than guessed at.
pub fn table_in_file(path: &str, wanted: &str) -> Result<Vec<Vec<f64>>, String> {
    // A URI that reached this far was never turned into a file: the
    // library it names is not one this compiler reads from. Saying so
    // by the places that were tried tells a missing file from a
    // library read from one place and its data looked for in another,
    // which are the same silence and not the same mistake.
    if path.starts_with("modelica://") {
        return Err(super::external::resource_at(path).unwrap_err());
    }
    let bytes = std::fs::read(path).map_err(|why| format!("`{path}` cannot be read: {why}"))?;
    // An editor that writes UTF-8 may put a mark for it in front of
    // everything else. It says nothing about the table and is not
    // part of the `#1` the format begins with, so it comes off here.
    let bytes = match bytes.strip_prefix(&[0xEF, 0xBB, 0xBF][..]) {
        Some(rest) => rest.to_vec(),
        None => bytes,
    };
    if bytes.starts_with(b"#1") {
        return in_text(&bytes, wanted, path);
    }
    // Level 5 says so in words, in the first line of a 128-byte
    // header: `MATLAB 5.0 MAT-file`. It is what MATLAB has written by
    // default for twenty years, so it is what the library's own data
    // is in, and level 4 is the older thing this compiler read first.
    if bytes.starts_with(b"MATLAB 5.0 MAT-file") {
        return in_matlab5(&bytes, wanted, path);
    }
    if looks_like_matlab(&bytes) {
        return in_matlab(&bytes, wanted, path);
    }
    // A file written on a machine that puts the high byte first says
    // so in the same word, read the other way round. Nothing here
    // swaps the bytes back, so it is refused by name rather than read
    // as a table of nonsense.
    if bytes.len() >= 20 {
        let swapped = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        if let Some((order, _, _)) = reads_as_a_header(swapped) {
            if order == 1 || swapped >= 1000 {
                return Err(format!(
                    "`{path}` is a MATLAB level 4 file of the other byte order, which this \
                     compiler does not read - `{wanted}` cannot be taken from it"
                ));
            }
        }
    }
    Err(format!(
        "`{path}` is neither a text table nor a MATLAB level 4 file, and those are the two \
         this compiler reads"
    ))
}

/// Whether a file's first bytes are a MATLAB level 4 header.
///
/// The header is five 32-bit numbers, and the first of them is read a
/// digit at a time: the thousands say the byte order, the tens the
/// precision, and the units whether the matrix holds numbers or text.
/// The hundreds are always nothing. A file written on an ordinary
/// machine has the thousands as nothing too, so what is left is a
/// number below a hundred whose hundreds digit is zero - which no
/// text table starts with, since a text table starts with `#1`.
fn looks_like_matlab(bytes: &[u8]) -> bool {
    if bytes.len() < 20 {
        return false;
    }
    let kind = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    reads_as_a_header(kind).is_some()
}

/// The digits of a level 4 header's first word, where they are ones
/// the format uses: the byte order, the precision, and whether the
/// matrix holds numbers or text. The hundreds are always nothing.
fn reads_as_a_header(kind: u32) -> Option<(u32, u32, u32)> {
    let (order, hundreds, precision, matrix) =
        (kind / 1000, (kind / 100) % 10, (kind / 10) % 10, kind % 10);
    match order <= 1 && hundreds == 0 && precision <= 5 && matrix <= 2 {
        true => Some((order, precision, matrix)),
        false => None,
    }
}

/// The table named in a text file.
///
/// The format is a line `double name(rows, columns)` and then that
/// many numbers, whitespace between them and `#` starting a comment
/// that runs to the end of its line. A file may hold several tables
/// one after another, and only the one asked for is read.
fn in_text(bytes: &[u8], wanted: &str, path: &str) -> Result<Vec<Vec<f64>>, String> {
    let text = String::from_utf8_lossy(bytes);
    let mut lines = text.lines();
    let mut found = None;
    while let Some(line) = lines.next() {
        // A header is `double name(rows, columns)`, and what follows
        // the closing bracket on that line is a comment.
        let Some(rest) = line.trim_start().strip_prefix("double ") else {
            continue;
        };
        let Some((named, shape)) = rest.split_once('(') else {
            continue;
        };
        if named.trim() != wanted {
            continue;
        }
        let Some((shape, _)) = shape.split_once(')') else {
            continue;
        };
        let Some((rows, columns)) = shape.split_once(',') else {
            return Err(format!(
                "`{wanted}` in `{path}` says its shape as `{shape}`, which is not rows and \
                 columns"
            ));
        };
        let count = |what: &str| -> Result<usize, String> {
            what.trim().parse::<usize>().map_err(|_| {
                format!("`{wanted}` in `{path}` says `{what}` where a whole number belongs")
            })
        };
        let (rows, columns) = (count(rows)?, count(columns)?);
        // The numbers themselves, taken from the lines that follow
        // until there are as many as the header promised. Comments
        // and blank lines are passed over.
        let mut numbers: Vec<f64> = Vec::with_capacity(rows * columns);
        for line in lines.by_ref() {
            let line = line.split('#').next().unwrap_or("");
            for word in line.split_whitespace() {
                let number = word.parse::<f64>().map_err(|_| {
                    format!("`{wanted}` in `{path}` has `{word}` where a number belongs")
                })?;
                numbers.push(number);
            }
            if numbers.len() >= rows * columns {
                break;
            }
        }
        if numbers.len() < rows * columns {
            return Err(format!(
                "`{wanted}` in `{path}` promises {rows} by {columns} and gives {} number(s)",
                numbers.len()
            ));
        }
        found = Some(
            numbers
                .chunks(columns.max(1))
                .take(rows)
                .map(<[f64]>::to_vec)
                .collect(),
        );
        break;
    }
    found.ok_or_else(|| format!("`{path}` holds no table called `{wanted}`"))
}

/// The table named in a MATLAB level 4 file.
///
/// The format is a run of matrices, each a header of five 32-bit
/// numbers, then the name, then the numbers themselves down the
/// columns. The header says how many rows and columns there are and
/// how the numbers are written; nothing is compressed and nothing
/// refers to anything else, which is what makes reading it worth
/// doing here rather than reaching for a library.
fn in_matlab(bytes: &[u8], wanted: &str, path: &str) -> Result<Vec<Vec<f64>>, String> {
    let mut at = 0usize;
    while at + 20 <= bytes.len() {
        let word = |k: usize| -> u32 {
            let start = at + k * 4;
            u32::from_le_bytes([
                bytes[start],
                bytes[start + 1],
                bytes[start + 2],
                bytes[start + 3],
            ])
        };
        let (kind, rows, columns, imaginary, name_length) = (
            word(0),
            word(1) as usize,
            word(2) as usize,
            word(3),
            word(4) as usize,
        );
        // The digits of the first number, from the top: the byte
        // order, the precision, and whether the matrix holds numbers
        // or text. Only a matrix of ordinary numbers is a table.
        let precision = (kind / 10) % 10;
        let matrix_kind = kind % 10;
        // The precision is one of six, and a header saying anything
        // else was refused as not being this format at all.
        let width = match precision {
            0 => 8,
            1 | 2 => 4,
            3 | 4 => 2,
            _ => 1,
        };
        let name_at = at + 20;
        let numbers_at = name_at + name_length;
        let held = rows * columns * (1 + imaginary as usize);
        let ends = numbers_at + held * width;
        if ends > bytes.len() {
            return Err(format!(
                "a matrix of `{path}` says it is {rows} by {columns} and reaches past the end \
                 of the file, so `{wanted}` cannot be taken from it"
            ));
        }
        // The name is written with a zero after it, the way C writes
        // a string.
        let name = String::from_utf8_lossy(&bytes[name_at..numbers_at])
            .trim_end_matches('\0')
            .to_string();
        if name == wanted {
            if matrix_kind != 0 {
                return Err(format!(
                    "`{wanted}` in `{path}` is text rather than a table of numbers"
                ));
            }
            let read = |k: usize| -> f64 {
                let start = numbers_at + k * width;
                let at = &bytes[start..start + width];
                match precision {
                    0 => f64::from_le_bytes(at.try_into().expect("eight bytes")),
                    1 => f32::from_le_bytes(at.try_into().expect("four bytes")) as f64,
                    2 => i32::from_le_bytes(at.try_into().expect("four bytes")) as f64,
                    3 => i16::from_le_bytes(at.try_into().expect("two bytes")) as f64,
                    4 => u16::from_le_bytes(at.try_into().expect("two bytes")) as f64,
                    _ => at[0] as f64,
                }
            };
            // MATLAB writes a matrix down its columns, and a table is
            // read across its rows.
            return Ok((0..rows)
                .map(|row| {
                    (0..columns)
                        .map(|column| read(column * rows + row))
                        .collect()
                })
                .collect());
        }
        at = ends;
    }
    Err(format!("`{path}` holds no table called `{wanted}`"))
}

/// A matrix out of a level 5 MATLAB file.
///
/// The shape of the format: a 128-byte header, then elements, each a
/// type and a length followed by that many bytes padded to eight. A
/// matrix is type 14 and holds four of those in turn - its flags and
/// class, its dimensions, its name, and its numbers - and the numbers
/// are column-major, which is the one thing worth saying twice.
///
/// Only what a table needs is read: a two-dimensional array of real
/// numbers, no complex part, no compression, little-endian. Anything
/// else is refused by name rather than guessed at.
fn in_matlab5(bytes: &[u8], wanted: &str, path: &str) -> Result<Vec<Vec<f64>>, String> {
    // `IM` in the two bytes after the version says the writer put the
    // low byte first, which is the only order read here.
    if bytes.len() < 132 || &bytes[126..128] != b"IM" {
        return Err(format!(
            "`{path}` is a MATLAB level 5 file this compiler cannot read - it is not \
             little-endian, and nothing here swaps the bytes back"
        ));
    }
    let word = |at: usize| -> Option<u32> {
        let held = bytes.get(at..at + 4)?;
        Some(u32::from_le_bytes([held[0], held[1], held[2], held[3]]))
    };
    // A tag is eight bytes, unless its top half is a length - the
    // format's own shorthand for something that fits in four.
    let tag = |at: usize| -> Option<(u32, usize, usize)> {
        let first = word(at)?;
        match first >> 16 {
            0 => Some((first & 0xffff, word(at + 4)? as usize, at + 8)),
            short => Some((first & 0xffff, short as usize, at + 4)),
        }
    };
    // What follows a tag, rounded up to the eight bytes the format
    // pads every element to.
    let past = |from: usize, length: usize, short: bool| match short {
        true => from + 4,
        false => from + length.div_ceil(8) * 8,
    };
    let mut at = 128;
    while let Some((kind, length, body)) = tag(at) {
        let element_past = past(body, length, body == at + 4);
        // 14 is a matrix; anything else at the top level is not one.
        if kind != 14 {
            at = element_past;
            continue;
        }
        let read = |mut at: usize| -> Option<(Vec<usize>, String, usize, usize, u32)> {
            // Array flags: the class is the low byte of the first of
            // the two words that follow.
            let (_, flags_length, flags) = tag(at)?;
            let class = word(flags)? & 0xff;
            at = past(flags, flags_length, false);
            // The dimensions, one 32-bit number each.
            let (_, dimensions_length, dimensions) = tag(at)?;
            let shape: Vec<usize> = (0..dimensions_length / 4)
                .filter_map(|which| word(dimensions + which * 4).map(|n| n as usize))
                .collect();
            at = past(dimensions, dimensions_length, dimensions == at + 4);
            // The name, as bytes.
            let (_, name_length, name_at) = tag(at)?;
            let name =
                String::from_utf8_lossy(bytes.get(name_at..name_at + name_length)?).into_owned();
            at = past(name_at, name_length, name_at == at + 4);
            // And the numbers.
            let (numbers_kind, numbers_length, numbers) = tag(at)?;
            Some((
                shape,
                name,
                numbers,
                numbers_length,
                numbers_kind | (class << 16),
            ))
        };
        if let Some((shape, name, numbers, numbers_length, marks)) = read(body) {
            if name == wanted {
                let (class, numbers_kind) = (marks >> 16, marks & 0xffff);
                // Class 6 is an array of doubles. What it is written
                // with is another matter: a matrix of whole numbers
                // small enough to fit is stored in the narrowest type
                // that holds them, which is the format saving space
                // rather than saying the numbers are not real. The
                // widths, by the format's own numbering.
                let width = match numbers_kind {
                    1 | 2 => Some(1),
                    3 | 4 => Some(2),
                    5 | 6 => Some(4),
                    7 => Some(4),
                    9 => Some(8),
                    12 | 13 => Some(8),
                    _ => None,
                };
                let (Some(width), 6) = (width, class) else {
                    return Err(format!(
                        "`{wanted}` in `{path}` is not an array of double precision numbers"
                    ));
                };
                let (rows, columns) = match shape.as_slice() {
                    [rows, columns] => (*rows, *columns),
                    _ => {
                        return Err(format!(
                            "`{wanted}` in `{path}` has {} dimension(s), and a table has two",
                            shape.len()
                        ))
                    }
                };
                if numbers_length < rows * columns * width {
                    return Err(format!(
                        "`{wanted}` in `{path}` says it is {rows} by {columns} and does not \
                         carry that many numbers"
                    ));
                }
                // Column-major on the file, row by row here, and
                // read by the width the numbers were written at.
                let held = |row: usize, column: usize| -> f64 {
                    let at = numbers + (column * rows + row) * width;
                    let of = |n: usize| bytes[at..at + n].to_vec();
                    match (numbers_kind, width) {
                        (1, _) => bytes[at] as i8 as f64,
                        (2, _) => bytes[at] as f64,
                        (3, _) => i16::from_le_bytes([of(2)[0], of(2)[1]]) as f64,
                        (4, _) => u16::from_le_bytes([of(2)[0], of(2)[1]]) as f64,
                        (5..=7, 4) => {
                            let held = of(4);
                            let four = [held[0], held[1], held[2], held[3]];
                            match numbers_kind {
                                5 => i32::from_le_bytes(four) as f64,
                                6 => u32::from_le_bytes(four) as f64,
                                _ => f32::from_le_bytes(four) as f64,
                            }
                        }
                        _ => {
                            let held = of(8);
                            let eight = [
                                held[0], held[1], held[2], held[3], held[4], held[5], held[6],
                                held[7],
                            ];
                            match numbers_kind {
                                12 => i64::from_le_bytes(eight) as f64,
                                13 => u64::from_le_bytes(eight) as f64,
                                _ => f64::from_le_bytes(eight),
                            }
                        }
                    }
                };
                return Ok((0..rows)
                    .map(|row| (0..columns).map(|column| held(row, column)).collect())
                    .collect());
            }
        }
        at = element_past;
    }
    Err(format!(
        "`{path}` is a MATLAB level 5 file with no table called `{wanted}` in it"
    ))
}
