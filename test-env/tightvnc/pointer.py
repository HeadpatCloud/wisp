#!/usr/bin/python3
from Xlib.display import Display

pointer = Display().screen().root.query_pointer()
print(f"x:{pointer.root_x} y:{pointer.root_y}")
