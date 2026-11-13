//! Streams the rows of an `.xlsx` sheet into Python as a lazy iterator.
//!
//! A reader thread pulls cells one at a time with calamine's `worksheet_cells_reader`, groups them
//! into rows and sends each row through a bounded channel. Python's `__next__` receives one row
//! (with the GIL released while it waits) and converts it to Python objects. Memory stays at the
//! shared-strings table plus a few rows, however large the sheet.

use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::thread;

use calamine::{Data, Reader, Xlsx, XlsxError, open_workbook};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::{IntoPyObjectExt, PyResult};

/// Rows the reader thread may run ahead of Python.
const CHANNEL_ROWS: usize = 1024;

type Row = Result<Vec<Data>, String>;

/// Iterator over the rows of a sheet. Each row is a `list`; a row without stored cells is `[]`,
/// so the n-th item is sheet row n (0-based, from row 1). Gaps inside a row are `None`.
#[pyclass]
struct RowIter {
    // Receiver is not Sync, and a pyclass must be.
    rx: Mutex<Receiver<Row>>,
}

#[pymethods]
impl RowIter {
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __next__(&self, py: Python<'_>) -> PyResult<Option<Vec<Py<PyAny>>>> {
        // Release the GIL while waiting so other Python threads keep running.
        let row = py.detach(|| self.rx.lock().unwrap().recv());
        match row {
            Err(_) => Ok(None), // reader thread finished: StopIteration
            Ok(Err(e)) => Err(PyValueError::new_err(e)),
            Ok(Ok(cells)) => cells.into_iter().map(|c| to_py(py, c)).collect::<PyResult<_>>().map(Some),
        }
    }
}

/// Streams the rows of `sheet` (default: the first sheet) of the `.xlsx` at `path`.
#[pyfunction]
#[pyo3(signature = (path, sheet=None))]
fn iter_rows(path: PathBuf, sheet: Option<String>) -> RowIter {
    let (tx, rx) = sync_channel(CHANNEL_ROWS);
    thread::spawn(move || {
        if let Err(e) = read_sheet(path, sheet, &tx) {
            let _ = tx.send(Err(e));
        }
    });
    RowIter { rx: Mutex::new(rx) }
}

/// Sends every row of the sheet; returns early (Ok) if Python dropped the iterator.
fn read_sheet(path: PathBuf, sheet: Option<String>, tx: &SyncSender<Row>) -> Result<(), String> {
    // .xlsb works the same way: calamine's `Xlsb` has the same `worksheet_cells_reader` /
    // `next_cell` API (row-major cells, `DataRef` values), so supporting it is a matter of opening
    // `Xlsb<BufReader<File>>` (with `XlsbError`) when the path ends in `.xlsb`; the loop is shared.
    let mut workbook: Xlsx<BufReader<File>> = open_workbook(&path).map_err(|e: XlsxError| e.to_string())?;
    let sheet = match sheet {
        Some(s) => s,
        None => workbook.sheet_names().first().cloned().ok_or("workbook has no sheets")?,
    };
    let mut cells = workbook.worksheet_cells_reader(&sheet).map_err(|e| e.to_string())?;

    let mut row: Vec<Data> = Vec::new();
    let mut current = 0u32;
    while let Some(cell) = cells.next_cell().map_err(|e| e.to_string())? {
        let (r, c) = cell.get_position();
        // Cells arrive in row-major order: flush the current row and any empty rows before r.
        while current < r {
            if tx.send(Ok(std::mem::take(&mut row))).is_err() {
                return Ok(());
            }
            current += 1;
        }
        let c = c as usize;
        if row.len() <= c {
            row.resize(c + 1, Data::Empty);
        }
        row[c] = cell.get_value().clone().into();
    }
    if !row.is_empty() {
        let _ = tx.send(Ok(row));
    }
    Ok(())
}

fn to_py(py: Python<'_>, cell: Data) -> PyResult<Py<PyAny>> {
    match cell {
        Data::Empty => Ok(py.None()),
        Data::Int(v) => v.into_py_any(py),
        Data::Float(v) => v.into_py_any(py),
        Data::Bool(v) => v.into_py_any(py),
        Data::String(v) | Data::DateTimeIso(v) | Data::DurationIso(v) => v.into_py_any(py),
        Data::Error(e) => e.to_string().into_py_any(py),
        Data::DateTime(dt) if dt.is_duration() => match dt.as_duration() {
            Some(d) => d.into_py_any(py),
            None => dt.as_f64().into_py_any(py),
        },
        Data::DateTime(dt) => match dt.as_datetime() {
            Some(d) => d.into_py_any(py),
            None => dt.as_f64().into_py_any(py),
        },
    }
}

#[pymodule]
fn calamine_stream(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<RowIter>()?;
    m.add_function(wrap_pyfunction!(iter_rows, m)?)
}
