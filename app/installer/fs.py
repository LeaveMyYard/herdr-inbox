"""App-home file operations. Every path is walked with no-follow directory descriptors."""
import hashlib
import os
import secrets
import shutil
import stat
from contextlib import contextmanager
from pathlib import PurePosixPath

import atomic


def digest(data):
    return hashlib.sha256(data).hexdigest()


def parts(path):
    pure = PurePosixPath(path)
    if not path or pure.is_absolute() or str(pure) != path or '..' in pure.parts:
        raise ValueError(f'Unsafe relative path: {path}')
    return pure.parts


class Home:
    def __init__(self, path):
        if not os.path.isabs(path):
            raise ValueError('--app-home must be absolute')
        # The operator named this folder, links on the way to it included. Inside it, none count.
        self.fd = os.open(os.path.realpath(path), os.O_RDONLY | os.O_DIRECTORY)

    @contextmanager
    def parent(self, path):
        fd = os.dup(self.fd)
        components = parts(path)
        try:
            for component in components[:-1]:
                child = os.open(component, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=fd)
                os.close(fd)
                fd = child
            yield fd, components[-1]
        finally:
            os.close(fd)

    def kind(self, path):
        """'file', 'dir', 'other' (a link, a socket), or None when nothing is there."""
        try:
            with self.parent(path) as (fd, name):
                mode = os.stat(name, dir_fd=fd, follow_symlinks=False).st_mode
        except FileNotFoundError:
            return None
        return 'file' if stat.S_ISREG(mode) else 'dir' if stat.S_ISDIR(mode) else 'other'

    def read(self, path, limit=None):
        """(bytes, mode) of a regular file, or None when there is none."""
        try:
            with self.parent(path) as (fd, name):
                child = os.open(name, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=fd)
        except FileNotFoundError:
            return None
        with os.fdopen(child, 'rb') as stream:
            info = os.fstat(stream.fileno())
            if not stat.S_ISREG(info.st_mode):
                raise ValueError(f'Not a regular file: {path}')
            if limit is not None and info.st_size > limit:
                raise ValueError(f'File too large: {path}')
            return stream.read(), stat.S_IMODE(info.st_mode)

    def list(self, path):
        with self.parent(path + '/.probe') as (fd, _):
            return sorted(os.listdir(fd))

    def walk(self, path):
        """Every path below a directory that is not one, depth first; links are not followed."""
        for name in self.list(path):
            child = f'{path}/{name}'
            if self.kind(child) == 'dir':
                yield from self.walk(child)
            else:
                yield child

    def mkdir(self, path, mode=0o755):
        """Creates the directory; False when it was there already."""
        with self.parent(path) as (fd, name):
            try:
                os.mkdir(name, mode, dir_fd=fd)
                return True
            except FileExistsError:
                os.close(os.open(name, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=fd))
                return False

    def chmod(self, path, mode):
        with self.parent(path) as (fd, name):
            child = os.open(name, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=fd)
        try:
            os.fchmod(child, mode)
        finally:
            os.close(child)

    def write(self, path, data, mode):
        """Publishes the whole file at once, replacing what is there."""
        with self.parent(path) as (fd, name):
            temp = '.herdr-inbox-' + secrets.token_hex(8)
            out = os.open(temp, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, mode, dir_fd=fd)
            try:
                with os.fdopen(out, 'wb') as stream:
                    stream.write(data)
                    stream.flush()
                    os.fchmod(stream.fileno(), mode)
                    os.fsync(stream.fileno())
                os.replace(temp, name, src_dir_fd=fd, dst_dir_fd=fd)
            except BaseException:
                os.unlink(temp, dir_fd=fd)
                raise
            os.fsync(fd)

    def touch(self, path):
        with self.parent(path) as (fd, name):
            os.close(os.open(name, os.O_WRONLY | os.O_CREAT | os.O_NOFOLLOW | os.O_NONBLOCK, 0o600, dir_fd=fd))

    def remove(self, path):
        try:
            with self.parent(path) as (fd, name):
                os.unlink(name, dir_fd=fd)
        except FileNotFoundError:
            pass

    def rmdir(self, path):
        """Removes an empty directory; False when something is left in it."""
        try:
            with self.parent(path) as (fd, name):
                os.rmdir(name, dir_fd=fd)
        except FileNotFoundError:
            pass
        except OSError:
            return False
        return True

    def rmtree(self, path):
        if self.kind(path) is not None:
            with self.parent(path) as (fd, name):
                shutil.rmtree(name, dir_fd=fd)

    def publish(self, source, target):
        """Moves directory source to target. A target already there comes back at source."""
        with self.parent(source) as (from_fd, from_name), self.parent(target) as (to_fd, to_name):
            move = atomic.swap if self.kind(target) is not None else atomic.rename_new
            move(from_fd, from_name, to_fd, to_name)
