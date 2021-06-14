# rjsonnet-py

![CI](https://github.com/messense/rjsonnet-py/workflows/CI/badge.svg)
[![PyPI](https://img.shields.io/pypi/v/rjsonnet.svg)](https://pypi.org/project/rjsonnet)

Python bindings to Rust [jrsonnet](https://github.com/CertainLach/jrsonnet) crates (Rust implementation of Jsonnet language).

## Installation

```bash
pip install rjsonnet
```

## Usage

This module provides two functions:

1. `def evaluate_file(filename: str) -> str: ...`
2. `def evaluate_snippet(filename: str, src: str) -> str: ...`

In the latter case, the parameter `filename` is used in stack traces,
because all errors are given with the "filename" containing the code.

Keyword arguments to these functions are used to control the virtual machine. They are:

* `max_stack`   (number)
* `gc_min_objects`   (number, ignored)
* `gc_growth_trigger`   (number, ignored)
* `ext_vars`   (dict: string to string)
* `ext_codes`   (dict string to string)
* `tla_vars`   (dict string to string)
* `tla_codes`   (dict string to string)
* `max_trace`   (number)
* `import_callback`   (see example in [tests/](./tests/))
* `native_callbacks`   (see example in [tests/](./tests/))

The argument `import_callback` can be used to pass a callable, to trap the Jsonnet `import` and `importstr` constructs.
This allows, e.g., reading files out of archives or implementing library search paths.

The argument `native_callbacks` is used to allow execution of arbitrary Python code via `std.native(...)`.
This is useful so Jsonnet code can access pure functions in the Python ecosystem, such as compression, encryption, encoding, etc.

If an error is raised during the evaluation of the Jsonnet code, it is formed into a stack trace and thrown as a python `RuntimeError`.

```python
import rjsonnet

# evaluate a jsonnet file
rjsonnet.evaluate_file("filename.jsonnet")

# evalute a jsonnet code snippet
rjsonnet.evaluate_snippet('filename', 'jsonnet code snippet')
```

## License

This work is released under the MIT license. A copy of the license is provided in the [LICENSE](./LICENSE) file.
