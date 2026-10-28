import unittest

class TestGreet(unittest.TestCase):
    def test_greet_ada(self):
        self.assertEqual(greet("Ada"), "Hello, Ada!")

    def test_greet_empty(self):
        self.assertEqual(greet(""), "Hello, !")

if __name__ == "__main__":
    unittest.main()