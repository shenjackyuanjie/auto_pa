import argparse


def add_argument(parser: argparse.ArgumentParser):
    parser.add_argument(
        "--skip-apps-check",
        "-s",
        action="store_true",
        help="Skip App Check",
        default=False,
    )
    parser.add_argument("--gallery-api", default="https://hmos.txit.top/api")
    parser.add_argument(
        "--fast-pull", action="store_true", help="Fast Pull", default=False
    )
    parser.add_argument(
        "--skip-app-categories",
        "-c",
        action="store_true",
        help="Skip App Categories",
        default=False,
    )
    parser.add_argument(
        "--skip-categories",
        "-k",
        help="Skip Categories",
        type=str,
        nargs="+",
        default=[],
    )
    parser.add_argument("--ping", type=int, help="Ping", default=5)
