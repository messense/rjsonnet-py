import rjsonnet


def test_evaluate_snippet():
    code = 'std.assertEqual(({ x: 1, y: self.x } { x: 2 }).y, 2)'
    assert rjsonnet.evaluate_snippet('test', code)
