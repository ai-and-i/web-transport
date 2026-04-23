"""Python 3.10 compat shims for asyncio features added in 3.11."""

from __future__ import annotations

import sys

if sys.version_info >= (3, 11):
    from asyncio import TaskGroup, timeout
else:
    from async_timeout import timeout
    from taskgroup import TaskGroup

__all__ = ["TaskGroup", "timeout"]
