import argparse
from .constant import add_argument as add_argument_base

def add_argument(
    parser: argparse.ArgumentParser,
):
    add_argument_base(parser)
    parser.add_argument(
        "--username",
        type=str,
        default="",
        help="Username for submit apps",
    )