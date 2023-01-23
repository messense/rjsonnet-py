use std::cell::RefCell;
use std::collections::HashMap;
use std::env;
use std::path::PathBuf;
use std::{any::Any, borrow::Cow};

use jrsonnet_evaluator::apply_tla;
use jrsonnet_evaluator::{
    error::{Error, ErrorKind::*},
    function::{
        builtin::{NativeCallback, NativeCallbackHandler},
        TlaArg,
    },
    gc::GcHashMap,
    manifest::{JsonFormat, ManifestFormat},
    stack::set_stack_depth_limit,
    tb,
    trace::{CompactFormat, PathResolver, TraceFormat},
    val::{ArrValue, StrValue},
    FileImportResolver, IStr, ImportResolver, ObjValue, State, Val,
};
use jrsonnet_gcmodule::Trace;
use jrsonnet_parser::{ParserSettings, Source, SourceDirectory, SourceFile, SourcePath};
use pyo3::exceptions::{PyRuntimeError, PyTypeError};
use pyo3::prelude::*;
use pyo3::types::{PyBool, PyDict, PyFloat, PyList, PySequence, PyString, PyTuple};

#[derive(Trace)]
struct PythonImportResolver {
    #[trace(skip)]
    callback: PyObject,
    out: RefCell<HashMap<SourcePath, Vec<u8>>>,
}

impl ImportResolver for PythonImportResolver {
    fn resolve_from(
        &self,
        from: &SourcePath,
        path: &str,
    ) -> jrsonnet_evaluator::error::Result<SourcePath> {
        let base = if let Some(file) = from.downcast_ref::<SourceFile>() {
            let mut path = file.path().to_path_buf();
            path.pop();
            path
        } else if let Some(dir) = from.downcast_ref::<SourceDirectory>() {
            dir.path().to_path_buf()
        } else if from.is_default() {
            env::current_dir().map_err(|e| ImportIo(e.to_string()))?
        } else {
            let err_msg = "can't resolve this path";
            Python::with_gil(|py| {
                let err = PyRuntimeError::new_err(err_msg);
                err.restore(py);
            });
            return Err(Error::new(ImportCallbackError(format!(
                "import_callback error: {}",
                err_msg
            ))));
        };
        let (resolved, content) =
            Python::with_gil(|py| match self.callback.call(py, (base, path), None) {
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
            let resolved = SourcePath::new(SourceFile::new(PathBuf::from(resolved)));
            let mut out = self.out.borrow_mut();
            if !out.contains_key(&resolved) {
                out.insert(resolved.clone(), content.into());
            }
            Ok(resolved)
        } else {
            Err(ImportFileNotFound(from.clone(), path.to_string()).into())
        }
    }

    fn load_file_contents(
        &self,
        resolved: &SourcePath,
    ) -> jrsonnet_evaluator::error::Result<Vec<u8>> {
        Ok(self.out.borrow().get(resolved).unwrap().clone())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn pyobject_to_val(py: Python, obj: PyObject) -> PyResult<Val> {
    return if let Ok(s) = obj.downcast::<PyString>(py) {
        s.to_str().map(|s| Val::Str(StrValue::Flat(s.into())))
    } else if let Ok(b) = obj.downcast::<PyBool>(py) {
        Ok(Val::Bool(b.is_true()))
    } else if let Ok(f) = obj.downcast::<PyFloat>(py) {
        Ok(Val::Num(f.value() as _))
    } else if let Ok(l) = obj.extract::<u64>(py) {
        Ok(Val::Num(l as _))
    } else if obj.is_none(py) {
        Ok(Val::Null)
    } else if let Ok(seq) = obj.downcast::<PySequence>(py) {
        let len = seq.len()?;
        let mut arr = Vec::with_capacity(len);
        for i in 0..len {
            let item = seq.get_item(i)?;
            arr.push(pyobject_to_val(py, item.into_py(py))?);
        }
        Ok(Val::Arr(ArrValue::eager(arr)))
    } else if let Ok(d) = obj.downcast::<PyDict>(py) {
        let mut map = ObjValue::new_empty();
        for (k, v) in d {
            let k = k.extract::<String>()?;
            let v = pyobject_to_val(py, v.into_py(py))?;
            map.extend_field(k.into()).value(v);
        }
        Ok(Val::Obj(map))
    } else {
        Err(PyTypeError::new_err(
            "Unrecognized type return from Python Jsonnet native extension.",
        ))
    };
}

fn val_to_pyobject(py: Python, val: &Val, preserve_order: bool) -> PyObject {
    match val {
        Val::Bool(b) => b.into_py(py),
        Val::Null => py.None(),
        Val::Str(s) => s.clone().into_flat().into_py(py),
        Val::Num(n) => n.into_py(py),
        Val::Arr(a) => {
            let arr = PyList::empty(py);
            for item in a.iter() {
                arr.append(val_to_pyobject(py, &item.unwrap(), preserve_order))
                    .unwrap();
            }
            arr.into_py(py)
        }
        Val::Obj(o) => {
            let dict = PyDict::new(py);
            for field in o.fields(preserve_order) {
                let k = field.to_string();
                let v = o
                    .get(field)
                    .unwrap()
                    .map(|x| val_to_pyobject(py, &x, preserve_order));
                dict.set_item(k, v).unwrap();
            }
            dict.into_py(py)
        }
        Val::Func(_) => unimplemented!(),
    }
}

#[derive(Trace)]
struct JsonnetNativeCallbackHandler {
    #[trace(skip)]
    name: String,
    #[trace(skip)]
    func: PyObject,
    #[trace(skip)]
    preserve_order: bool,
}

impl NativeCallbackHandler for JsonnetNativeCallbackHandler {
    fn call(&self, args: &[Val]) -> Result<Val, Error> {
        Python::with_gil(|py| {
            let args: Vec<_> = args
                .iter()
                .map(|v| val_to_pyobject(py, v, self.preserve_order))
                .collect();
            let err = match self.func.call(py, PyTuple::new(py, args), None) {
                Ok(obj) => match pyobject_to_val(py, obj) {
                    Ok(val) => return Ok(val),
                    Err(err) => err,
                },
                Err(err) => err,
            };
            let err_msg = err.to_string();
            err.restore(py);
            Err(Error::new(RuntimeError(
                format!("error invoking native extension {}: {}", self.name, err_msg).into(),
            )))
        })
    }
}

struct VirtualMachine {
    state: State,
    manifest_format: Box<dyn ManifestFormat>,
    trace_format: Box<dyn TraceFormat>,
    tla_args: GcHashMap<IStr, TlaArg>,
}

impl VirtualMachine {
    #[allow(clippy::too_many_arguments)]
    #[inline]
    fn new(
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
        preserve_order: bool,
    ) -> PyResult<Self> {
        let state = State::default();
        set_stack_depth_limit(max_stack);

        state.settings_mut().import_resolver = tb!(FileImportResolver::default());

        let trace_format = CompactFormat {
            max_trace,
            ..Default::default()
        };

        let context_initializer = jrsonnet_stdlib::ContextInitializer::new(
            state.clone(),
            PathResolver::new_cwd_fallback(),
        );

        for (k, v) in ext_vars.into_iter() {
            context_initializer.add_ext_str(k.into(), v.into());
        }
        for (k, v) in ext_codes.into_iter() {
            context_initializer
                .add_ext_code(k.as_str(), v.as_str())
                .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
        }

        let mut tla_args = GcHashMap::new();

        for (k, v) in tla_vars.into_iter() {
            tla_args.insert(k.into(), TlaArg::String(v.into()));
        }
        for (k, v) in tla_codes.into_iter() {
            let name: IStr = k.into();
            let code: IStr = v.clone().into();
            let code = jrsonnet_parser::parse(
                &v,
                &ParserSettings {
                    source: Source::new_virtual(format!("<top-level-arg:{name}>").into(), code),
                },
            )
            .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
            tla_args.insert(name, TlaArg::Code(code));
        }

        if let Some(import_callback) = import_callback {
            if !import_callback.as_ref(py).is_callable() {
                return Err(PyTypeError::new_err("import_callback must be callable"));
            }
            let import_resolver = PythonImportResolver {
                callback: import_callback,
                out: RefCell::new(HashMap::new()),
            };
            state.set_import_resolver(import_resolver);
        } else if let Some(jpathdir) = jpathdir {
            let import_resolver = FileImportResolver::new(jpathdir);
            state.set_import_resolver(import_resolver);
        }

        for (name, (args, func)) in native_callbacks.into_iter() {
            let args = args.downcast::<PyTuple>(py)?;
            let mut params = Vec::with_capacity(args.len());
            for arg in args {
                let param = arg.extract::<String>()?;
                params.push(Cow::Owned(param));
            }
            context_initializer.add_native(
                name.clone().into(),
                #[allow(deprecated)]
                NativeCallback::new(
                    params,
                    JsonnetNativeCallbackHandler {
                        name,
                        func,
                        preserve_order,
                    },
                ),
            );
        }

        state.settings_mut().context_initializer = tb!(context_initializer);
        Ok(Self {
            state,
            manifest_format: Box::new(JsonFormat::default()),
            trace_format: Box::new(trace_format),
            tla_args,
        })
    }

    fn evaluate_file(&self, filename: &str) -> Result<String, Error> {
        self.state
            .import_from(&SourcePath::new(SourceDirectory::new(".".into())), filename)
            .and_then(|val| apply_tla(self.state.clone(), &self.tla_args, val))
            .and_then(|val| val.manifest(&self.manifest_format))
    }

    fn evaluate_snippet(&self, filename: &str, snippet: &str) -> Result<String, Error> {
        self.state
            .evaluate_snippet(filename, snippet)
            .and_then(|val| apply_tla(self.state.clone(), &self.tla_args, val))
            .and_then(|val| val.manifest(&self.manifest_format))
    }

    fn error_to_pyerr(&self, py: Python, err: &Error) -> PyErr {
        let cause = if PyErr::occurred(py) {
            Some(PyErr::fetch(py))
        } else {
            None
        };
        let mut err_msg = String::new();
        self.trace_format.write_trace(&mut err_msg, err).unwrap();
        let py_err = PyRuntimeError::new_err(err_msg);
        if cause.is_some() {
            py_err.set_cause(py, cause);
        }
        py_err
    }
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
#[pyfunction(signature = (
    filename,
    jpathdir = None,
    max_stack = 500,
    gc_min_objects = 1000,
    gc_growth_trigger = 2.0,
    ext_vars = HashMap::new(),
    ext_codes = HashMap::new(),
    tla_vars = HashMap::new(),
    tla_codes = HashMap::new(),
    max_trace = 20,
    import_callback = None,
    native_callbacks = HashMap::new(),
    preserve_order = false,
))]
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
    preserve_order: bool,
) -> PyResult<String> {
    let vm = VirtualMachine::new(
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
        preserve_order,
    )?;

    let result = vm
        .evaluate_file(filename)
        .map_err(|e| vm.error_to_pyerr(py, &e))?;
    Ok(result)
}

/// Evaluate jsonnet code snippet
#[allow(clippy::too_many_arguments)]
#[pyfunction(signature = (
    filename,
    src,
    jpathdir = None,
    max_stack = 500,
    gc_min_objects = 1000,
    gc_growth_trigger = 2.0,
    ext_vars = HashMap::new(),
    ext_codes = HashMap::new(),
    tla_vars = HashMap::new(),
    tla_codes = HashMap::new(),
    max_trace = 20,
    import_callback = None,
    native_callbacks = HashMap::new(),
    preserve_order = false,
))]
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
    preserve_order: bool,
) -> PyResult<String> {
    let vm = VirtualMachine::new(
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
        preserve_order,
    )?;

    let result = vm
        .evaluate_snippet(filename, src)
        .map_err(|e| vm.error_to_pyerr(py, &e))?;
    Ok(result)
}

/// Python bindings to Rust jrsonnet crate
#[pymodule]
fn rjsonnet(_py: Python, m: &PyModule) -> PyResult<()> {
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    m.add_function(wrap_pyfunction!(evaluate_file, m)?)?;
    m.add_function(wrap_pyfunction!(evaluate_snippet, m)?)?;
    Ok(())
}
