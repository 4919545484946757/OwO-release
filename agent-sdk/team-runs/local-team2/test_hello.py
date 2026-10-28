import unittest

class TestGreet(unittest.TestCase):
    def test_greet(self):
        self.assertEqual(greet("Ada"), "Hello, Ada!")
        self.assertEqual(greet(""), "Hello, !")

if __name__ == "__main__":
    unittest.main()