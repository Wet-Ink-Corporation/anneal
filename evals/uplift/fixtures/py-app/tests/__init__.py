"""Puts `src` on the import path, so `python3 -m unittest` finds the code."""

import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(os.path.dirname(__file__)), "src"))
