try:
    from importlib.metadata import version as _version

    __version__ = _version("herdr-inbox")
except Exception:  # noqa: BLE001 - not installed as a package
    __version__ = "0"
