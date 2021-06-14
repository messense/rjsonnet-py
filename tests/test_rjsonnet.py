import os

import rjsonnet


def test_evaluate_file():
    def import_callback(dir, rel):
        current_dir = os.path.abspath(os.path.dirname(__file__))
        return os.path.join(current_dir, rel), None

    assert rjsonnet.evaluate_file("test.jsonnet", import_callback=import_callback)


def test_evaluate_snippet():
    code = 'std.assertEqual(({ x: 1, y: self.x } { x: 2 }).y, 2)'
    assert rjsonnet.evaluate_snippet('test', code)
