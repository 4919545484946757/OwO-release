import unittest

import calc
class T(unittest.TestCase):
    def test_add(self):
        self.assertEqual(calc.add(1, 2), 3)

if __name__ == '__main__':
    unittest.main()