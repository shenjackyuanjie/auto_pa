import argparse
from .constant import add_argument as add_argument_base

def add_argument(
    parser: argparse.ArgumentParser,
):
    add_argument_base(parser)
    parser.add_argument(
        "--fast-pull", action="store_true", help="Fast Pull", default=False
    )

__all__ = [
    "add_argument"
]