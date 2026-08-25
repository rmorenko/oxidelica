#!/bin/sh
# Hand the question document to a fresh session of the consulting
# model, let it read the code, and have it append its answer to the
# same document. The document is the memory: every question and answer
# stays in it, so a later consultation sees the earlier ones.
#
# Write the question into QUESTION_FOR_FABLE.md first - symptom, a
# small reproduction, the trace, what was already tried - then run
# this. The answer is appended to the document and printed.
set -eu
cd "$(dirname "$0")/.."

test -f QUESTION_FOR_FABLE.md || {
    echo "QUESTION_FOR_FABLE.md not found: write the question first" >&2
    exit 1
}

exec claude --model claude-fable-5 \
    --permission-mode acceptEdits \
    -p "Read QUESTION_FOR_FABLE.md in this repository. The last section \
is a question that has no answer section after it yet. Before \
answering, study the code the question names - the paths in it are \
real paths in this repository - and verify the claims against the \
source rather than taking them on faith. Then append your answer to \
QUESTION_FOR_FABLE.md as a new section after a '---' separator, its \
heading matching the answer sections already in the document, written \
in the language the question was asked in. Do not modify any file \
other than QUESTION_FOR_FABLE.md. Finally, print the full answer."
