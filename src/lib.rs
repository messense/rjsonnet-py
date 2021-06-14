use std::any::Any;
use std::fs::File;
use std::io::prelude::*;
use std::path::PathBuf;
use std::rc::Rc;

use jrsonnet_evaluator::{EvaluationState, ImportResolver, Val};
use jrsonnet_interner::IStr;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};
use pyo3::wrap_pyfunction;

struct PythonImportResolver {
    callback: PyObject,
}

impl ImportResolver for PythonImportResolver {
    fn resolve_file(
        &self,
        from: &PathBuf,
        path: &PathBuf,
    ) -> jrsonnet_evaluator::error::Result<Rc<PathBuf>> {
        use jrsonnet_evaluator::error::Error::*;

        // FIXME: use PathBuf directly on PyO3 0.14
        let from_str = from.to_str().unwrap();
        let path_str = path.to_str().unwrap();
        let resolved =
            Python::with_gil(
                |py| match self.callback.call(py, (from_str, path_str), None) {
                    Ok(obj) => {
                        if let Ok((resolved, _content)) = obj.extract::<(String, Option<&str>)>(py)
                        {
                            Ok(resolved)
                        } else {
                            Err(ImportFileNotFound(from.clone(), path.clone()))
                        }
                    }
                    Err(_) => Err(ImportFileNotFound(from.clone(), path.clone())),
                },
            )?;
        Ok(Rc::new(PathBuf::from(resolved)))
    }

    fn load_file_contents(&self, resolved: &PathBuf) -> jrsonnet_evaluator::error::Result<IStr> {
        use jrsonnet_evaluator::error::Error::*;

        let mut file = File::open(resolved).map_err(|_e| ResolvedFileNotFound(resolved.clone()))?;
        let mut out = String::new();
        file.read_to_string(&mut out)
            .map_err(|_e| ImportBadFileUtf8(resolved.clone()))?;
        Ok(out.into())
    }

    unsafe fn as_any(&self) -> &dyn Any {
        panic!("this resolver can't be used as any")
    }
}

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

#[pyfunction(import_callback = "None")]
fn evaluate_file(
    py: Python,
    filename: &str,
    import_callback: Option<PyObject>,
) -> PyResult<PyObject> {
    let path = PathBuf::from(filename);
    let state = EvaluationState::default();

    if let Some(import_callback) = import_callback {
        let import_resolver = PythonImportResolver {
            callback: import_callback,
        };
        state.set_import_resolver(Box::new(import_resolver));
    }

    let result = state.with_stdlib().evaluate_file_raw(&path).unwrap();
    Ok(val_to_pyobject(py, result))
}

#[pyfunction(import_callback = "None")]
fn evaluate_snippet(
    py: Python,
    filename: &str,
    expr: &str,
    import_callback: Option<PyObject>,
) -> PyResult<PyObject> {
    let path = PathBuf::from(filename);
    let state = EvaluationState::default();

    if let Some(import_callback) = import_callback {
        let import_resolver = PythonImportResolver {
            callback: import_callback,
        };
        state.set_import_resolver(Box::new(import_resolver));
    }

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
