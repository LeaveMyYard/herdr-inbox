"""Atomic directory publication: neither call can leave a mod half replaced."""
import ctypes
import os
import sys

# Linux: include/uapi/linux/fs.h. macOS: sys/stdio.h.
FLAGS = {'linux': ('renameat2', 1, 2), 'darwin': ('renameatx_np', 4, 2)}


def _rename(source_fd, source, target_fd, target, which):
    if sys.platform not in FLAGS:
        raise ValueError('Installing needs Linux or macOS')
    name, *flags = FLAGS[sys.platform]
    function = getattr(ctypes.CDLL(None, use_errno=True), name)
    function.argtypes = [ctypes.c_int, ctypes.c_char_p, ctypes.c_int, ctypes.c_char_p, ctypes.c_uint]
    function.restype = ctypes.c_int
    if function(source_fd, os.fsencode(source), target_fd, os.fsencode(target), flags[which]) != 0:
        error = ctypes.get_errno()
        raise OSError(error, os.strerror(error), target)


def rename_new(source_fd, source, target_fd, target):
    """Moves source to target, failing if target exists."""
    _rename(source_fd, source, target_fd, target, 0)


def swap(source_fd, source, target_fd, target):
    """Exchanges source and target, both of which must exist."""
    _rename(source_fd, source, target_fd, target, 1)
