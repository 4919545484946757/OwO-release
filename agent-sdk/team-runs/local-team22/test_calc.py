import unittest
import calc

class TestAdd(unittest.TestCase):
    def test_add(self):
        self.assertEqual(calc.add(2, 3), 5)
        self.assertEqual(calc.add(-1, 1), 0

if __name__ == '__main__':
    unittest.main()