use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use jrsonnet_evaluator::error::LocError;
use jrsonnet_evaluator::native::{NativeCallback, NativeCallbackHandler};
use jrsonnet_evaluator::{
    ArrValue, EvaluationState, FileImportResolver, IStr, ImportResolver, LazyBinding, LazyVal,
    ObjMember, ObjValue, Val,
};
use jrsonnet_gc::{unsafe_empty_trace, Finalize, Gc, Trace};
use jrsonnet_parser::{Param, ParamsDesc, Visibility};
use pyo3::exceptions::{PyRuntimeError, PyTypeError};
use pyo3::prelude::*;
use pyo3::types::{PyBool, PyDict, PyFloat, PyList, PySequence, PyString, PyTuple};

struct PythonImportResolver {
    callback: PyObject,
    out: RefCell<HashMap<PathBuf, IStr>>,
}

impl ImportResolver for PythonImportResolver {
    fn resolve_file(
        &self,
        from: &Path,
        path: &Path,
    ) -> jrsonnet_evaluator::error::Result<Rc<Path>> {
        use jrsonnet_evaluator::error::Error::*;

        let (resolved, content) =
            Python::with_gil(|py| match self.callback.call(py, (from, path), None) {
                Ok(obj) => obj.extract::<(String, Option<String>)>(py).map_err(|err| {
                    let err_msg = err.to_string();
                    err.restore(py);
                    ImportCallbackError(format!("import_callback error: {}", err_msg))
                }),
                Err(err) => {
                    let err_msg = err.to_string();
                    err.restore(py);
                    Err(ImportCallbackError(format!(
                        "import_callback error: {}",
                        err_msg
                    )))
                }
            })?;
        if let Some(content) = content {
            let resolved = PathBuf::from(resolved);
            let mut out = self.out.borrow_mut();
            if !out.contains_key(&resolved) {
                out.insert(resolved.clone(), content.into());
            }
            Ok(resolved.into())
        } else {
            Err(ImportFileNotFound(from.to_path_buf(), path.to_path_buf()).into())
        }
    }

    fn load_file_contents(&self, resolved: &Path) -> jrsonnet_evaluator::error::Result<IStr> {
        Ok(self.out.borrow().get(resolved).unwrap().clone())
    }

    unsafe fn as_any(&self) -> &dyn Any {
        panic!("this resolver can't be used as any")
    }
}

fn pyobject_to_val(py: Python, obj: PyObject) -> PyResult<Val> {
    return if let Ok(s) = obj.cast_as::<PyString>(py) {
        s.to_str().map(|s| Val::Str(s.into()))
    } else if let Ok(b) = obj.cast_as::<PyBool>(py) {
        Ok(Val::Bool(b.is_true()))
    } else if let Ok(f) = obj.cast_as::<PyFloat>(py) {
        Ok(Val::Num(f.value() as _))
    } else if let Ok(l) = obj.extract::<u64>(py) {
        Ok(Val::Num(l as _))
    } else if obj.is_none(py) {
        Ok(Val::Null)
    } else if let Ok(seq) = obj.cast_as::<PySequence>(py) {
        let len = seq.len()?;
        let mut arr = Vec::with_capacity(len as usize);
        for i in 0..len {
            let item = seq.get_item(i)?;
            arr.push(pyobject_to_val(py, item.into_py(py))?);
        }
        Ok(Val::Arr(ArrValue::Eager(Gc::new(arr))))
    } else if let Ok(d) = obj.cast_as::<PyDict>(py) {
        let mut map = ObjValue::new_empty();
        for (k, v) in d {
            let k = k.extract::<String>()?;
            let v = pyobject_to_val(py, v.into_py(py))?;
            map = map.extend_with_field(
                k.into(),
                ObjMember {
                    add: false,
                    visibility: Visibility::Normal,
                    invoke: LazyBinding::Bound(LazyVal::new_resolved(v)),
                    location: None,
                },
            );
        }
        Ok(Val::Obj(map))
    } else {
        Err(PyTypeError::new_err(
            "Unrecognized type return from Python Jsonnet native extension.",
        ))
    };
}

fn val_to_pyobject(py: Python, val: &Val) -> PyObject {
    match val {
        Val::Bool(b) => b.into_py(py),
        Val::Null => py.None(),
        Val::Str(s) => s.into_py(py),
        Val::Num(n) => n.into_py(py),
        Val::Arr(a) => {
            let arr = PyList::empty(py);
            for item in a.iter() {
                arr.append(val_to_pyobject(py, &item.unwrap())).unwrap();
            }
            arr.into_py(py)
        }
        Val::Obj(o) => {
            let dict = PyDict::new(py);
            for field in o.fields() {
                let k = field.to_string();
                let v = o.get(field).unwrap().map(|x| val_to_pyobject(py, &x));
                dict.set_item(k, v).unwrap();
            }
            dict.into_py(py)
        }
        Val::Func(_) => unimplemented!(),
    }
}

struct JsonnetNativeCallbackHandler {
    name: String,
    func: PyObject,
}

impl Finalize for JsonnetNativeCallbackHandler {}

unsafe impl Trace for JsonnetNativeCallbackHandler {
    unsafe_empty_trace!();
}

impl NativeCallbackHandler for JsonnetNativeCallbackHandler {
    fn call(&self, _from: Option<Rc<Path>>, args: &[Val]) -> Result<Val, LocError> {
        Python::with_gil(|py| {
            let args: Vec<_> = args.iter().map(|v| val_to_pyobject(py, v)).collect();
            let err = match self.func.call(py, PyTuple::new(py, args), None) {
                Ok(obj) => match pyobject_to_val(py, obj) {
                    Ok(val) => return Ok(val),
                    Err(err) => err,
                },
                Err(err) => err,
            };
            let err_msg = err.to_string();
            err.restore(py);
            Err(LocError::new(
                jrsonnet_evaluator::error::Error::RuntimeError(
                    format!("error invoking native extension {}: {}", self.name, err_msg).into(),
                ),
            ))
        })
    }
}

#[allow(clippy::too_many_arguments)]
#[inline]
fn create_evaluation_state(
    py: Python,
    jpathdir: Option<Vec<PathBuf>>,
    max_stack: usize,
    ext_vars: HashMap<String, String>,
    ext_codes: HashMap<String, String>,
    tla_vars: HashMap<String, String>,
    tla_codes: HashMap<String, String>,
    max_trace: usize,
    import_callback: Option<PyObject>,
    native_callbacks: HashMap<String, (PyObject, PyObject)>,
) -> PyResult<EvaluationState> {
    let vm = EvaluationState::default();
    vm.set_max_stack(max_stack);
    vm.set_max_trace(max_trace);
    for (k, v) in ext_vars.into_iter() {
        vm.add_ext_str(k.into(), v.into());
    }
    for (k, v) in ext_codes.into_iter() {
        vm.add_ext_code(k.into(), v.into())
            .map_err(|e| PyRuntimeError::new_err(vm.stringify_err(&e)))?;
    }
    for (k, v) in tla_vars.into_iter() {
        vm.add_tla_str(k.into(), v.into());
    }
    for (k, v) in tla_codes.into_iter() {
        vm.add_tla_code(k.into(), v.into())
            .map_err(|e| PyRuntimeError::new_err(vm.stringify_err(&e)))?;
    }

    if let Some(import_callback) = import_callback {
        if !import_callback.as_ref(py).is_callable() {
            return Err(PyTypeError::new_err("import_callback must be callable"));
        }
        let import_resolver = PythonImportResolver {
            callback: import_callback,
            out: RefCell::new(HashMap::new()),
        };
        vm.set_import_resolver(Box::new(import_resolver));
    } else if let Some(jpathdir) = jpathdir {
        let import_resolver = FileImportResolver {
            library_paths: jpathdir,
        };
        vm.set_import_resolver(Box::new(import_resolver));
    }

    for (name, (args, func)) in native_callbacks.into_iter() {
        let args = args.cast_as::<PyTuple>(py)?;
        let mut params = Vec::with_capacity(args.len());
        for arg in args {
            let param = arg.extract::<&str>()?;
            params.push(Param(param.into(), None));
        }
        let params = ParamsDesc(Rc::new(params));
        vm.add_native(
            name.clone().into(),
            Gc::new(NativeCallback::new(
                params,
                Box::new(JsonnetNativeCallbackHandler { name, func }),
            )),
        );
    }
    Ok(vm)
}

fn loc_error_to_pyerr(py: Python, vm: &EvaluationState, loc_err: &LocError) -> PyErr {
    let cause = if PyErr::occurred(py) {
        Some(PyErr::fetch(py))
    } else {
        None
    };
    let py_err = PyRuntimeError::new_err(vm.stringify_err(loc_err));
    if cause.is_some() {
        py_err.set_cause(py, cause);
    }
    py_err
}

#[derive(FromPyObject)]
enum LibraryPath {
    Single(PathBuf),
    Multi(Vec<PathBuf>),
}

impl LibraryPath {
    fn into_vec(self) -> Vec<PathBuf> {
        match self {
            LibraryPath::Single(s) => vec![s],
            LibraryPath::Multi(l) => l,
        }
    }
}

/// Evaluate jsonnet file
#[allow(clippy::too_many_arguments)]
#[pyfunction(
    jpathdir = "None",
    max_stack = "500",
    gc_min_objects = "1000",
    gc_growth_trigger = "2.0",
    ext_vars = "HashMap::new()",
    ext_codes = "HashMap::new()",
    tla_vars = "HashMap::new()",
    tla_codes = "HashMap::new()",
    max_trace = "20",
    import_callback = "None",
    native_callbacks = "HashMap::new()"
)]
fn evaluate_file(
    py: Python,
    filename: &str,
    jpathdir: Option<LibraryPath>,
    max_stack: usize,
    #[allow(unused_variables)] gc_min_objects: usize,
    #[allow(unused_variables)] gc_growth_trigger: f64,
    ext_vars: HashMap<String, String>,
    ext_codes: HashMap<String, String>,
    tla_vars: HashMap<String, String>,
    tla_codes: HashMap<String, String>,
    max_trace: usize,
    import_callback: Option<PyObject>,
    native_callbacks: HashMap<String, (PyObject, PyObject)>,
) -> PyResult<String> {
    let path = PathBuf::from(filename);
    let vm = create_evaluation_state(
        py,
        jpathdir.map(|x| x.into_vec()),
        max_stack,
        ext_vars,
        ext_codes,
        tla_vars,
        tla_codes,
        max_trace,
        import_callback,
        native_callbacks,
    )?;

    let result = vm
        .with_stdlib()
        .evaluate_file_raw_nocwd(&path)
        .and_then(|v| vm.with_tla(v))
        .and_then(|v| vm.manifest(v))
        .map_err(|e| loc_error_to_pyerr(py, &vm, &e))?;
    Ok(result.to_string())
}

/// Evaluate jsonnet code snippet
#[allow(clippy::too_many_arguments)]
#[pyfunction(
    jpathdir = "None",
    max_stack = "500",
    gc_min_objects = "1000",
    gc_growth_trigger = "2.0",
    ext_vars = "HashMap::new()",
    ext_codes = "HashMap::new()",
    tla_vars = "HashMap::new()",
    tla_codes = "HashMap::new()",
    max_trace = "20",
    import_callback = "None",
    native_callbacks = "HashMap::new()"
)]
fn evaluate_snippet(
    py: Python,
    filename: &str,
    src: &str,
    jpathdir: Option<LibraryPath>,
    max_stack: usize,
    #[allow(unused_variables)] gc_min_objects: usize,
    #[allow(unused_variables)] gc_growth_trigger: f64,
    ext_vars: HashMap<String, String>,
    ext_codes: HashMap<String, String>,
    tla_vars: HashMap<String, String>,
    tla_codes: HashMap<String, String>,
    max_trace: usize,
    import_callback: Option<PyObject>,
    native_callbacks: HashMap<String, (PyObject, PyObject)>,
) -> PyResult<String> {
    let path = PathBuf::from(filename);
    let vm = create_evaluation_state(
        py,
        jpathdir.map(|x| x.into_vec()),
        max_stack,
        ext_vars,
        ext_codes,
        tla_vars,
        tla_codes,
        max_trace,
        import_callback,
        native_callbacks,
    )?;

    let result = vm
        .with_stdlib()
        .evaluate_snippet_raw(path.into(), src.into())
        .and_then(|v| vm.with_tla(v))
        .and_then(|v| vm.manifest(v))
        .map_err(|e| loc_error_to_pyerr(py, &vm, &e))?;
    Ok(result.to_string())
}

/// Python bindings to Rust jrsonnet crate
#[pymodule]
fn rjsonnet(_py: Python, m: &PyModule) -> PyResult<()> {
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    m.add_function(wrap_pyfunction!(evaluate_file, m)?)?;
    m.add_function(wrap_pyfunction!(evaluate_snippet, m)?)?;
    Ok(())
}
