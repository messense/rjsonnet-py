use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

use jrsonnet_evaluator::{EvaluationState, ImportResolver, Val};
use jrsonnet_interner::IStr;
use pyo3::exceptions::{PyNotImplementedError, PyRuntimeError};
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};
use pyo3::wrap_pyfunction;

struct PythonImportResolver {
    callback: PyObject,
    out: RefCell<HashMap<PathBuf, IStr>>,
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
        let (resolved, content) =
            Python::with_gil(
                |py| match self.callback.call(py, (from_str, path_str), None) {
                    Ok(obj) => obj
                        .extract::<(String, Option<String>)>(py)
                        .map_err(|_e| ImportFileNotFound(from.clone(), path.clone())),
                    Err(_) => Err(ImportFileNotFound(from.clone(), path.clone())),
                },
            )?;
        if let Some(content) = content {
            let resolved = PathBuf::from(resolved);
            let mut out = self.out.borrow_mut();
            if !out.contains_key(&resolved) {
                out.insert(resolved.clone(), content.into());
            }
            Ok(Rc::new(resolved))
        } else {
            Err(ImportFileNotFound(from.clone(), path.clone()).into())
        }
    }

    fn load_file_contents(&self, resolved: &PathBuf) -> jrsonnet_evaluator::error::Result<IStr> {
        Ok(self.out.borrow().get(resolved).unwrap().clone())
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

#[inline]
fn create_evaluation_state(
    max_stack: usize,
    max_trace: usize,
    ext_vars: HashMap<String, String>,
    ext_codes: HashMap<String, String>,
    tla_vars: HashMap<String, String>,
    tla_codes: HashMap<String, String>,
    import_callback: Option<PyObject>,
    native_callbacks: Option<&PyDict>,
) -> PyResult<EvaluationState> {
    if native_callbacks.is_some() {
        return Err(PyNotImplementedError::new_err(
            "native_callbacks not implemented yet",
        ));
    }
    let state = EvaluationState::default();
    state.set_max_stack(max_stack);
    state.set_max_trace(max_trace);
    for (k, v) in ext_vars.into_iter() {
        state.add_ext_str(k.into(), v.into());
    }
    for (k, v) in ext_codes.into_iter() {
        state
            .add_ext_code(k.into(), v.into())
            .map_err(|e| PyRuntimeError::new_err(format!("add_ext_code error: {:?}", e)))?;
    }
    for (k, v) in tla_vars.into_iter() {
        state.add_tla_str(k.into(), v.into());
    }
    for (k, v) in tla_codes.into_iter() {
        state
            .add_tla_code(k.into(), v.into())
            .map_err(|e| PyRuntimeError::new_err(format!("add_tla_code error: {:?}", e)))?;
    }

    if let Some(import_callback) = import_callback {
        let import_resolver = PythonImportResolver {
            callback: import_callback,
            out: RefCell::new(HashMap::new()),
        };
        state.set_import_resolver(Box::new(import_resolver));
    }
    Ok(state)
}

/// Evaluate jsonnet file
#[pyfunction(
    max_stack = "500",
    max_trace = "20",
    gc_min_objects = "1000",
    gc_growth_trigger = "2.0",
    ext_vars = "HashMap::new()",
    ext_codes = "HashMap::new()",
    tla_vars = "HashMap::new()",
    tla_codes = "HashMap::new()",
    import_callback = "None",
    native_callbacks = "None"
)]
fn evaluate_file(
    py: Python,
    filename: &str,
    max_stack: usize,
    max_trace: usize,
    #[allow(unused_variables)] gc_min_objects: usize,
    #[allow(unused_variables)] gc_growth_trigger: f64,
    ext_vars: HashMap<String, String>,
    ext_codes: HashMap<String, String>,
    tla_vars: HashMap<String, String>,
    tla_codes: HashMap<String, String>,
    import_callback: Option<PyObject>,
    native_callbacks: Option<&PyDict>,
) -> PyResult<PyObject> {
    let path = PathBuf::from(filename);
    let state = create_evaluation_state(
        max_stack,
        max_trace,
        ext_vars,
        ext_codes,
        tla_vars,
        tla_codes,
        import_callback,
        native_callbacks,
    )?;

    let result = state
        .with_stdlib()
        .evaluate_file_raw(&path)
        .map_err(|e| PyRuntimeError::new_err(format!("evaluate_file error: {:?}", e)))?;
    Ok(val_to_pyobject(py, result))
}

/// Evaluate jsonnet code snippet
#[pyfunction(
    max_stack = "500",
    max_trace = "20",
    gc_min_objects = "1000",
    gc_growth_trigger = "2.0",
    ext_vars = "HashMap::new()",
    ext_codes = "HashMap::new()",
    tla_vars = "HashMap::new()",
    tla_codes = "HashMap::new()",
    import_callback = "None",
    native_callbacks = "None"
)]
fn evaluate_snippet(
    py: Python,
    filename: &str,
    expr: &str,
    max_stack: usize,
    max_trace: usize,
    #[allow(unused_variables)] gc_min_objects: usize,
    #[allow(unused_variables)] gc_growth_trigger: f64,
    ext_vars: HashMap<String, String>,
    ext_codes: HashMap<String, String>,
    tla_vars: HashMap<String, String>,
    tla_codes: HashMap<String, String>,
    import_callback: Option<PyObject>,
    native_callbacks: Option<&PyDict>,
) -> PyResult<PyObject> {
    let path = PathBuf::from(filename);
    let state = create_evaluation_state(
        max_stack,
        max_trace,
        ext_vars,
        ext_codes,
        tla_vars,
        tla_codes,
        import_callback,
        native_callbacks,
    )?;

    let result = state
        .with_stdlib()
        .evaluate_snippet_raw(Rc::new(path), expr.into())
        .map_err(|e| PyRuntimeError::new_err(format!("evaluate_snippet error: {:?}", e)))?;
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
