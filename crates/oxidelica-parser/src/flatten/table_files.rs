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
    table_in_file_as(path, wanted, &Csv::default())
}

/// How a comma-separated file is to be read: what stands between the
/// numbers, and how many lines at the top are not numbers at all. The
/// table blocks hand both to their constructor beside the file name,
/// and the defaults are theirs.
#[derive(Debug, Clone, PartialEq)]
pub struct Csv {
    /// The character between two numbers, beside the blanks that
    /// always separate them.
    pub delimiter: char,
    /// How many lines at the top are passed over unread.
    pub header_lines: usize,
}

impl Default for Csv {
    fn default() -> Self {
        Csv {
            delimiter: ',',
            header_lines: 0,
        }
    }
}

impl Csv {
    /// The reading a table block asked for, from the two parameters it
    /// names them by. A delimiter that is not one character is refused
    /// by the library's own reader, and so it is not guessed at here.
    pub fn asked(delimiter: Option<String>, header_lines: Option<f64>) -> Option<Csv> {
        let delimiter = match delimiter {
            None => ',',
            Some(text) => {
                let mut characters = text.chars();
                match (characters.next(), characters.next()) {
                    (Some(one), None) => one,
                    _ => return None,
                }
            }
        };
        let header_lines = match header_lines {
            None => 0,
            Some(count) if count >= 0.0 && count.fract() == 0.0 => count as usize,
            Some(_) => return None,
        };
        Some(Csv {
            delimiter,
            header_lines,
        })
    }
}

/// The same, with the file read as comma-separated where its name says
/// it is - which is how the standard library's reader tells, by the
/// extension and nothing else. A comma-separated file holds one table
/// and gives it no name, so the name asked for is not looked at.
pub fn table_in_file_as(path: &str, wanted: &str, csv: &Csv) -> Result<Vec<Vec<f64>>, String> {
    if is_csv(path) && !path.starts_with("modelica://") {
        let bytes = std::fs::read(path).map_err(|why| format!("`{path}` cannot be read: {why}"))?;
        return in_csv(&bytes, csv, path);
    }
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

/// The switch back to how tables on files were read before the series
/// that taught the reader comma-separated files, fields of a MATLAB
/// structure and the unpadded compressed elements of version 7, and
/// taught the measurement to take a range by its bounds. Kept so that
/// the corpus can be run twice from one binary.
pub(super) fn old_file_tables() -> bool {
    std::env::var_os("OXIDELICA_OLD_FILE_TABLES").is_some()
}

/// Whether a file is comma-separated, told the way the standard
/// library's reader tells it: by the extension.
pub(super) fn is_csv(path: &str) -> bool {
    !old_file_tables()
        && std::path::Path::new(path)
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("csv"))
}

/// The table in a comma-separated file.
///
/// Read as the standard library's own reader reads it: the header lines
/// are passed over, every line after them is a row, and a row is split
/// on the delimiter and on blanks alike. The width is the first row's,
/// and a row that is short of it, or a field that is not a number, is
/// refused by its line rather than filled with anything.
fn in_csv(bytes: &[u8], csv: &Csv, path: &str) -> Result<Vec<Vec<f64>>, String> {
    let text = String::from_utf8_lossy(bytes);
    let splits = |c: char| c == ' ' || c == '\t' || c == '\r' || c == csv.delimiter;
    let mut rows: Vec<Vec<f64>> = Vec::new();
    for (index, line) in text.lines().enumerate().skip(csv.header_lines) {
        let row = line
            .split(splits)
            .filter(|field| !field.is_empty())
            .map(|field| {
                field.parse::<f64>().map_err(|_| {
                    format!(
                        "line {} of `{path}` has `{field}` where a number belongs",
                        index + 1
                    )
                })
            })
            .collect::<Result<Vec<f64>, String>>()?;
        if let Some(first) = rows.first() {
            if row.len() < first.len() {
                return Err(format!(
                    "line {} of `{path}` has {} number(s) where the first row had {}",
                    index + 1,
                    row.len(),
                    first.len()
                ));
            }
        }
        let width = rows.first().map_or(row.len(), Vec::len);
        rows.push(row.into_iter().take(width).collect());
    }
    match rows.first().is_some_and(|row| !row.is_empty()) {
        true => Ok(rows),
        false => Err(format!("`{path}` holds no numbers after its header")),
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
    // MATLAB version 7 writes the same elements as version 6, each one
    // deflated and wrapped in an element of its own - type 15, the
    // format's `miCOMPRESSED`. A reader that skips what it does not
    // recognise sees a file with no matrix in it at all, which is what
    // left every `test_v7.mat` model refused at a flexible size with
    // nowhere to read a length from. Unpacked here, in front of the
    // walk, so that the walk goes on being about the level 5 format
    // and not about how a particular version stored it.
    if let Some(plain) = uncompressed(bytes) {
        return in_matlab5_walk(&plain, wanted, path);
    }
    in_matlab5_walk(bytes, wanted, path)
}

/// A level 5 file with its deflated elements unpacked, where it has
/// any.
///
/// The header is kept as it stands and every element after it is
/// copied through: a compressed one is inflated and what comes out is
/// a run of ordinary elements, so the result is a file of the same
/// format that nothing downstream has to know about. `None` where no
/// element is compressed, so that a version 6 file pays nothing.
fn uncompressed(bytes: &[u8]) -> Option<Vec<u8>> {
    let word = |at: usize| -> Option<u32> {
        let held = bytes.get(at..at + 4)?;
        Some(u32::from_le_bytes([held[0], held[1], held[2], held[3]]))
    };
    let mut out = bytes.get(..128)?.to_vec();
    let mut at = 128;
    let mut found = false;
    while let Some(kind) = word(at) {
        let (kind, length, body) = match kind >> 16 {
            0 => (kind & 0xffff, word(at + 4)? as usize, at + 8),
            short => (kind & 0xffff, short as usize, at + 4),
        };
        let past = match (body == at + 4, kind) {
            (true, _) => body + 4,
            // A compressed element is not padded to eight: MATLAB
            // writes the next one straight after the stream, so a
            // reader that rounds up lands in the middle of it and sees
            // one table where the file holds three.
            (false, 15) if !old_file_tables() => body + length,
            (false, _) => body + length.div_ceil(8) * 8,
        };
        // 15 is `miCOMPRESSED`: a zlib stream whose contents are the
        // elements the writer would otherwise have written plainly.
        match kind == 15 {
            true => {
                use std::io::Read;
                let held = bytes.get(body..body + length)?;
                let mut plain = Vec::new();
                flate2::read::ZlibDecoder::new(held)
                    .read_to_end(&mut plain)
                    .ok()?;
                out.extend_from_slice(&plain);
                found = true;
            }
            false => out.extend_from_slice(bytes.get(at..past.min(bytes.len()))?),
        }
        at = past;
        if past <= 128 {
            return None;
        }
    }
    found.then_some(out)
}

/// The walk over a level 5 file's elements, with nothing compressed
/// left in it.
///
/// A name with dots in it - `s.s.tab1` - is a table kept inside a
/// structure, which is how the standard library's own test files keep
/// some of theirs: the first part names a variable at the top of the
/// file, and each part after it a field one structure further down.
fn in_matlab5_walk(bytes: &[u8], wanted: &str, path: &str) -> Result<Vec<Vec<f64>>, String> {
    let walk = Level5 { bytes };
    let mut parts = wanted.split('.');
    let top = match old_file_tables() {
        true => wanted,
        false => parts.next().unwrap_or(wanted),
    };
    let fields: Vec<&str> = match old_file_tables() {
        true => Vec::new(),
        false => parts.collect(),
    };
    let mut at = 128;
    while let Some((kind, length, body)) = walk.tag(at) {
        let element_past = walk.past(body, length, body == at + 4);
        // 14 is a matrix; anything else at the top level is not one.
        if kind == 14 {
            if let Some(matrix) = walk.matrix(body) {
                if matrix.name == top {
                    return walk.descend(matrix, &fields, wanted, path);
                }
            }
        }
        at = element_past;
    }
    Err(format!(
        "`{path}` is a MATLAB level 5 file with no table called `{wanted}` in it"
    ))
}

/// The bytes of a level 5 file, and the few readings every element
/// needs.
struct Level5<'a> {
    bytes: &'a [u8],
}

/// A matrix element read as far as its name: what class of array it
/// is, its dimensions, and where what follows the name begins.
struct Matrix {
    class: u32,
    shape: Vec<usize>,
    name: String,
    rest: usize,
}

impl Level5<'_> {
    fn word(&self, at: usize) -> Option<u32> {
        let held = self.bytes.get(at..at + 4)?;
        Some(u32::from_le_bytes([held[0], held[1], held[2], held[3]]))
    }

    /// A tag is eight bytes, unless its top half is a length - the
    /// format's own shorthand for something that fits in four.
    fn tag(&self, at: usize) -> Option<(u32, usize, usize)> {
        let first = self.word(at)?;
        match first >> 16 {
            0 => Some((first & 0xffff, self.word(at + 4)? as usize, at + 8)),
            short => Some((first & 0xffff, short as usize, at + 4)),
        }
    }

    /// What follows a tag, rounded up to the eight bytes the format
    /// pads every element to.
    fn past(&self, from: usize, length: usize, short: bool) -> usize {
        match short {
            true => from + 4,
            false => from + length.div_ceil(8) * 8,
        }
    }

    /// The flags, dimensions and name every matrix starts with.
    fn matrix(&self, mut at: usize) -> Option<Matrix> {
        // Array flags: the class is the low byte of the first of the
        // two words that follow.
        let (_, flags_length, flags) = self.tag(at)?;
        let class = self.word(flags)? & 0xff;
        at = self.past(flags, flags_length, false);
        // The dimensions, one 32-bit number each.
        let (_, dimensions_length, dimensions) = self.tag(at)?;
        let shape: Vec<usize> = (0..dimensions_length / 4)
            .filter_map(|which| self.word(dimensions + which * 4).map(|n| n as usize))
            .collect();
        at = self.past(dimensions, dimensions_length, dimensions == at + 4);
        // The name, as bytes.
        let (_, name_length, name_at) = self.tag(at)?;
        let name =
            String::from_utf8_lossy(self.bytes.get(name_at..name_at + name_length)?).into_owned();
        at = self.past(name_at, name_length, name_at == at + 4);
        Some(Matrix {
            class,
            shape,
            name,
            rest: at,
        })
    }

    /// The table a matrix holds, or the one a field of it holds when
    /// there are fields left to go down.
    fn descend(
        &self,
        matrix: Matrix,
        fields: &[&str],
        wanted: &str,
        path: &str,
    ) -> Result<Vec<Vec<f64>>, String> {
        let Some((field, further)) = fields.split_first() else {
            return self.numbers(&matrix, wanted, path);
        };
        // Class 2 is a structure. Only a single one is read: an array
        // of structures would need an index the name does not give.
        if matrix.class != 2 || matrix.shape.iter().product::<usize>() != 1 {
            return Err(format!(
                "`{wanted}` in `{path}` goes through `{}`, which is not a single structure",
                matrix.name
            ));
        }
        let missing = || format!("`{path}` holds no field `{field}` on the way to `{wanted}`");
        // After the name, a structure says how long each field name is,
        // then gives the names end to end at that length, then one
        // matrix per field in the same order.
        let (_, _, width_at) = self.tag(matrix.rest).ok_or_else(missing)?;
        let width = self.word(width_at).ok_or_else(missing)? as usize;
        let at = self.past(width_at, 4, width_at == matrix.rest + 4);
        let (_, names_length, names_at) = self.tag(at).ok_or_else(missing)?;
        let names = self
            .bytes
            .get(names_at..names_at + names_length)
            .ok_or_else(missing)?;
        let which = names
            .chunks(width.max(1))
            .position(|name| {
                let end = name
                    .iter()
                    .position(|byte| *byte == 0)
                    .unwrap_or(name.len());
                &name[..end] == field.as_bytes()
            })
            .ok_or_else(missing)?;
        let mut at = self.past(names_at, names_length, names_at == at + 4);
        for _ in 0..which {
            let (_, length, body) = self.tag(at).ok_or_else(missing)?;
            at = self.past(body, length, body == at + 4);
        }
        let (kind, _, body) = self.tag(at).ok_or_else(missing)?;
        if kind != 14 {
            return Err(missing());
        }
        let inner = self.matrix(body).ok_or_else(missing)?;
        self.descend(inner, further, wanted, path)
    }

    /// The numbers of a matrix of doubles, row by row.
    fn numbers(&self, matrix: &Matrix, wanted: &str, path: &str) -> Result<Vec<Vec<f64>>, String> {
        let bytes = self.bytes;
        let not_numbers =
            || format!("`{wanted}` in `{path}` is not an array of double precision numbers");
        let (numbers_kind, numbers_length, numbers) =
            self.tag(matrix.rest).ok_or_else(not_numbers)?;
        // Class 6 is an array of doubles. What it is written with is
        // another matter: a matrix of whole numbers small enough to fit
        // is stored in the narrowest type that holds them, which is the
        // format saving space rather than saying the numbers are not
        // real. The widths, by the format's own numbering.
        let width = match numbers_kind {
            1 | 2 => Some(1),
            3 | 4 => Some(2),
            5 | 6 => Some(4),
            7 => Some(4),
            9 => Some(8),
            12 | 13 => Some(8),
            _ => None,
        };
        let (Some(width), 6) = (width, matrix.class) else {
            return Err(not_numbers());
        };
        let (rows, columns) = match matrix.shape.as_slice() {
            [rows, columns] => (*rows, *columns),
            _ => {
                return Err(format!(
                    "`{wanted}` in `{path}` has {} dimension(s), and a table has two",
                    matrix.shape.len()
                ))
            }
        };
        if numbers_length < rows * columns * width || bytes.len() < numbers + rows * columns * width
        {
            return Err(format!(
                "`{wanted}` in `{path}` says it is {rows} by {columns} and does not carry that \
                 many numbers"
            ));
        }
        // Column-major on the file, row by row here, and read by the
        // width the numbers were written at.
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
                        held[0], held[1], held[2], held[3], held[4], held[5], held[6], held[7],
                    ];
                    match numbers_kind {
                        12 => i64::from_le_bytes(eight) as f64,
                        13 => u64::from_le_bytes(eight) as f64,
                        _ => f64::from_le_bytes(eight),
                    }
                }
            }
        };
        Ok((0..rows)
            .map(|row| (0..columns).map(|column| held(row, column)).collect())
            .collect())
    }
}
