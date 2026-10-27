import unittest

import hello

class TestGreet(unittest.TestCase):
    def test_greet(self):
        self.assertEqual(hello.greet("Ada"), "Hello, Ada!")
        self.assertEqual(hello.greet(""), "Hello, !")

if __name__ == "__main__":
    unittest.main()