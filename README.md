# calamine-stream

Read `.xlsx` sheets from Python **row by row**, with [calamine](https://crates.io/crates/calamine)'s streaming cell reader doing the parsing in Rust, exposed through [PyO3](https://pyo3.rs).

```python
from calamine_stream import iter_rows

for row in iter_rows("big.xlsx", sheet="Data"):  # sheet defaults to the first one
    print(row)  # ['name', 1, 2.5, True, datetime.datetime(2026, 1, 2, 3, 4, 5)]
```

The whole sheet is never in memory: a 1,000,000-row workbook streams in about 1 s with a 16 MB peak RSS (Apple M-series).

## Requirements

- CPython 3.10 or newer. The extension is built against the stable ABI (`abi3`), so one build works on every CPython from 3.10 up.
- A Rust toolchain ([rustup](https://rustup.rs)) on any machine that builds it from source. The package is not on PyPI and there are no prebuilt wheels, so installing from git or from a source checkout compiles it (about a minute the first time).
- Tested on macOS (Apple Silicon). Linux and Windows should work since calamine and PyO3 support them, but have not been tested.

## Installing it in your project

Pick one.

**From git** (needs Rust on the installing machine):

```bash
uv add "calamine-stream @ git+https://github.com/Luisetepe/python-calamine-example"
# or
pip install "git+https://github.com/Luisetepe/python-calamine-example"
```

To pin a version, add `@<tag-or-commit>` to the URL.

**From a local checkout** (needs Rust):

```bash
uv add ../python-calamine-example        # or: pip install ../python-calamine-example
```

**From a wheel** (no Rust needed where it is installed): build the wheel once, then install that file.

```bash
cd python-calamine-example
uv build --wheel                 # writes dist/calamine_stream-<version>-cp310-abi3-<platform>.whl
pip install dist/calamine_stream-*.whl
```

A wheel only installs on the OS and CPU it was built on (the `<platform>` tag in its name, e.g. `macosx_11_0_arm64`). For Linux servers or Windows machines, build it on that platform, or in CI with [maturin-action](https://github.com/PyO3/maturin-action).

The distribution is named `calamine-stream`; the module you import is `calamine_stream`.

## API

### `iter_rows(path, sheet=None)`

Returns an iterator over the rows of one sheet of an `.xlsx` workbook.

| Parameter | Type | Meaning |
|-----------|------|---------|
| `path` | `str` or `os.PathLike` | Path to the `.xlsx` file |
| `sheet` | `str` or `None` | Exact sheet name (case-sensitive, hidden sheets included). `None` reads the first sheet in workbook order |

Each item is a `list` of cell values for one sheet row:

| Excel cell | Python value |
|------------|--------------|
| Empty / not stored | `None` |
| Number | `float`, whole numbers included (`1` → `1.0`): `.xlsx` stores every number as a float. Use `int(v)` where you need an integer |
| Text | `str` |
| Boolean | `bool` |
| Date / time | `datetime.datetime` |
| Duration (`[h]:mm` formats) | `datetime.timedelta` |
| Error (`#DIV/0!`, `#N/A`, …) | `str` with the error text |
| Formula | Its cached result, as above (formulas are not evaluated) |

Row shape:

- The n-th item (0-based) is always sheet row n+1. A row with no stored cells comes back as `[]`, so you can count rows with `enumerate`.
- A row is as wide as its last stored cell, so rows can have different lengths. A gap inside a row is `None`. Pad rows yourself if you need a fixed width.
- Iteration ends at the last stored row; trailing blank rows are not returned.

Errors: a missing file, a missing sheet or a corrupt workbook raises `ValueError` on the **first `next()`**, not on the `iter_rows(...)` call. Wrap the loop, not the call:

```python
try:
    for row in iter_rows(path, sheet="Data"):
        ...
except ValueError as e:
    print(f"could not read {path}: {e}")
```

Reading stops as soon as you stop iterating: breaking out of the loop (or dropping the iterator) stops the reader thread, so reading only the header row of a huge file is cheap.

## Common patterns

Header row + dicts:

```python
rows = iter_rows("book.xlsx", sheet="Data")
header = next(rows)
for row in rows:
    record = dict(zip(header, row))   # zip stops at the shorter list: missing trailing cells are dropped
```

Batches, e.g. for a database insert:

```python
from itertools import batched   # Python 3.12+

for batch in batched(iter_rows("book.xlsx"), 5_000):
    cursor.executemany("INSERT INTO t VALUES (?, ?, ?)", batch)
```

Into pandas or polars (this builds the whole frame in memory, so streaming only helps while reading):

```python
import pandas as pd

rows = iter_rows("book.xlsx")
df = pd.DataFrame(rows, columns=next(rows))
```

## How it works

Python does not have to hand the whole file to Rust; a PyO3 class can be a normal Python iterator (`__iter__` / `__next__`).

calamine's `worksheet_cells_reader` borrows the workbook, and a `#[pyclass]` can't hold a borrow. So `iter_rows` starts a reader thread that owns the workbook, groups cells into rows and sends them through a bounded channel (1,024 rows). Each `__next__` takes one row, with the GIL released while it waits, and converts it to Python objects. Memory is the workbook's shared-strings table plus at most ~1,000 rows, whatever the sheet size.

## Limits

- `.xlsx` only. `.xlsb` would work the same way (calamine's `.xlsb` reader has the same streaming API; see the comment in `src/lib.rs`), `.xls` and `.ods` have no streaming reader in calamine.
- No function to list sheet names yet: pass the name you expect, or omit it for the first sheet.
- Values are calamine's raw cell values, not Excel's displayed text: number formats (`0.00`, `%`, currency) are not applied.
- No type stub (`.pyi`) yet, so type checkers see `iter_rows` as untyped.

## Development

You need [uv](https://docs.astral.sh/uv/) and a Rust toolchain. uv builds the extension through maturin and rebuilds it when `src/*.rs` changes.

```bash
uv run pytest            # build + test
uv build                 # sdist + wheel in dist/
```

| Path | What |
|------|------|
| `src/lib.rs` | The whole extension: reader thread, row grouping, Python conversion |
| `tests/test_iter_rows.py` | Tests on workbooks generated with openpyxl |
| `pyproject.toml` | Python package metadata, maturin build backend |
| `Cargo.toml` | Rust crate: `pyo3` (abi3, chrono) and `calamine` |
