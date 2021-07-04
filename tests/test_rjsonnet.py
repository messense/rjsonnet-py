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
    assert (
        rjsonnet.evaluate_file(
            "test.jsonnet",
            import_callback=import_callback,
            native_callbacks=native_callbacks,
        )
        == "true"
    )

    assert (
        rjsonnet.evaluate_file(
            "test.jsonnet",
            jpathdir=os.path.abspath(os.path.dirname(__file__)),
            native_callbacks=native_callbacks,
        )
        == "true"
    )

    assert (
        rjsonnet.evaluate_file(
            "test.jsonnet",
            jpathdir=[os.path.abspath(os.path.dirname(__file__))],
            native_callbacks=native_callbacks,
        )
        == "true"
    )

    bad_native_callbacks = native_callbacks.copy()
    bad_native_callbacks["concat"] = (("a", "b"), lambda a: a)
    with pytest.raises(RuntimeError) as exc:
        rjsonnet.evaluate_file(
            "test.jsonnet",
            jpathdir=os.path.abspath(os.path.dirname(__file__)),
            native_callbacks=bad_native_callbacks,
        )
    assert isinstance(exc.value.__cause__, TypeError)


def test_evaluate_snippet():
    code = "std.assertEqual(({ x: 1, y: self.x } { x: 2 }).y, 2)"
    assert rjsonnet.evaluate_snippet("test", code) == "true"


def test_import_callback_non_callable():
    with pytest.raises(TypeError):
        rjsonnet.evaluate_file(
            "test.jsonnet",
            import_callback="bad import callback",
            native_callbacks=native_callbacks,
        )


def test_import_callback_error():
    def import_callback_1(dir, rel):
        raise ValueError("error")

    with pytest.raises(RuntimeError) as exc:
        rjsonnet.evaluate_file(
            "test.jsonnet",
            import_callback=import_callback_1,
            native_callbacks=native_callbacks,
        )
    assert isinstance(exc.value.__cause__, ValueError)

    def import_callback_2(dir, rel):
        return "fake", None

    with pytest.raises(RuntimeError):
        rjsonnet.evaluate_file(
            "test.jsonnet",
            import_callback=import_callback_2,
            native_callbacks=native_callbacks,
        )
