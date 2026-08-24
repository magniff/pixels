# Vim keys

## Motions

| key | moves |
| --- | ----- |
| h j k l | left down up right |
| w b e | by word |
| 0 $ | line start, line end |
| gg G | file start, file end |

## Operators

Operators combine with motions, so `d2w` deletes two words and
`c$` changes to the end of the line.

- `d` delete
- `c` change
- `y` yank

Doubling one makes it linewise: `dd`, `cc`, `yy`.

## Not implemented

Linewise visual mode, blockwise visual mode, marks, macros,
registers other than the unnamed one, and search.
