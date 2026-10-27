import unittest
from hello import greet
class TestGreet(unittest.TestCase):
    def test_ada(self):
        self.assertEqual(greet("Ada"), "Hello, Ada!")
    def test_empty(self):
        self.assertEqual(greet(""), "Hello, !")

if __name__ == "__main__":
    unittest.main()