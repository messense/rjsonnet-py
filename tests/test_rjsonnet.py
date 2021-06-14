import os

import pytest
import rjsonnet


def import_callback(dir, rel):
    current_dir = os.path.abspath(os.path.dirname(__file__))
    path = os.path.join(current_dir, rel)
    with open(path, "r") as f:
        return path, f.read()


# Test native extensions
def concat(a, b):
    return a + b


def return_types():
    return {
        "a": [1, 2, 3, None, []],
        "b": 1.0,
        "c": True,
        "d": None,
        "e": {"x": 1, "y": 2, "z": ["foo"]},
    }


native_callbacks = {
    "concat": (("a", "b"), concat),
    "return_types": ((), return_types),
}


def test_evaluate_file():
    assert rjsonnet.evaluate_file(
        "test.jsonnet",
        import_callback=import_callback,
        native_callbacks=native_callbacks,
    )

    assert rjsonnet.evaluate_file(
        "test.jsonnet",
        jpathdir=os.path.abspath(os.path.dirname(__file__)),
        native_callbacks=native_callbacks,
    )

    assert rjsonnet.evaluate_file(
        "test.jsonnet",
        jpathdir=[os.path.abspath(os.path.dirname(__file__))],
        native_callbacks=native_callbacks,
    )

    bad_native_callbacks = native_callbacks.copy()
    bad_native_callbacks["concat"] = (("a", "b"), lambda a: a)
    with pytest.raises(RuntimeError):
        rjsonnet.evaluate_file(
            "test.jsonnet",
            jpathdir=os.path.abspath(os.path.dirname(__file__)),
            native_callbacks=bad_native_callbacks,
        )


def test_evaluate_snippet():
    code = "std.assertEqual(({ x: 1, y: self.x } { x: 2 }).y, 2)"
    assert rjsonnet.evaluate_snippet("test", code)
