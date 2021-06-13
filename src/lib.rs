use std::path::PathBuf;
use std::rc::Rc;

use jrsonnet_evaluator::{EvaluationState, Val};
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};
use pyo3::wrap_pyfunction;

fn val_to_pyobject(py: Python, val: Val) -> PyObject {
    match val {
        Val::Bool(b) => b.into_py(py),
        Val::Null => py.None(),
        Val::Str(s) => s.into_py(py),
        Val::Num(n) => n.into_py(py),
        Val::Arr(a) => {
            let arr = PyList::empty(py);
            for item in a.iter() {
                arr.append(val_to_pyobject(py, item.unwrap())).unwrap();
            }
            arr.into_py(py)
        }
        Val::Obj(o) => {
            let dict = PyDict::new(py);
            for field in o.fields() {
                let k = field.to_string();
                let v = o.get(field).unwrap().map(|x| val_to_pyobject(py, x));
                dict.set_item(k, v).unwrap();
            }
            dict.into_py(py)
        }
        Val::Func(_) => unimplemented!(),
    }
}

#[pyfunction]
fn evaluate_file(py: Python, filename: &str) -> PyResult<PyObject> {
    let path = PathBuf::from(filename);
    let state = EvaluationState::default();
    let result = state.with_stdlib().evaluate_file_raw(&path).unwrap();
    Ok(val_to_pyobject(py, result))
}

#[pyfunction]
fn evaluate_snippet(py: Python, filename: &str, expr: &str) -> PyResult<PyObject> {
    let path = PathBuf::from(filename);
    let state = EvaluationState::default();
    let result = state
        .with_stdlib()
        .evaluate_snippet_raw(Rc::new(path), expr.into())
        .unwrap();
    Ok(val_to_pyobject(py, result))
}

/// Python bindings to Rust jrsonnet crate
#[pymodule]
fn rjsonnet(_py: Python, m: &PyModule) -> PyResult<()> {
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    m.add_function(wrap_pyfunction!(evaluate_file, m)?)?;
    m.add_function(wrap_pyfunction!(evaluate_snippet, m)?)?;
    Ok(())
}
