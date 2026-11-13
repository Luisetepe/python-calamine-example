import datetime as dt

import openpyxl
import pytest

from calamine_stream import iter_rows


@pytest.fixture
def book(tmp_path):
    wb = openpyxl.Workbook()
    ws = wb.active
    ws.title = "Data"
    ws.append(["name", "qty", "price", "ok", "when"])
    ws.append(["a", 1, 2.5, True, dt.datetime(2026, 1, 2, 3, 4, 5)])
    ws["C4"] = "after a blank row"
    wb.create_sheet("Other")["A1"] = "other"
    path = tmp_path / "book.xlsx"
    wb.save(path)
    return path


def test_streams_rows_lazily(book):
    rows = iter_rows(book)
    assert next(rows) == ["name", "qty", "price", "ok", "when"]
    assert next(rows) == ["a", 1, 2.5, True, dt.datetime(2026, 1, 2, 3, 4, 5)]
    assert next(rows) == []  # row 3 has no cells
    assert next(rows) == [None, None, "after a blank row"]
    with pytest.raises(StopIteration):
        next(rows)


def test_named_sheet(book):
    assert list(iter_rows(str(book), sheet="Other")) == [["other"]]


def test_errors_raise_on_iteration(book):
    with pytest.raises(ValueError):
        list(iter_rows(book, sheet="missing"))


def test_early_break_does_not_hang(book):
    for _ in iter_rows(book):
        break
