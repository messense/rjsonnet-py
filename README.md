# rjsonnet-py

![CI](https://github.com/messense/rjsonnet-py/workflows/CI/badge.svg)
[![PyPI](https://img.shields.io/pypi/v/rjsonnet.svg)](https://pypi.org/project/rjsonnet)

Python bindings to Rust [jrsonnet](https://github.com/CertainLach/jrsonnet) crate

## Installation

```bash
pip install rjsonnet
```

## Usage

```python
import rjsonnet

# evaluate a jsonnet file
rjsonnet.evaluate_file("filename.jsonnet")

# evalute a jsonnet code snippet
rjsonnet.evaluate_snippet('filename', 'jsonnet code snippet')
```

## License

This work is released under the MIT license. A copy of the license is provided in the [LICENSE](./LICENSE) file.
