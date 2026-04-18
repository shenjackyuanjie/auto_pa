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
    parser.add_argument("--ping", type=int, help="Ping", default=15)
    parser.add_argument("--repeated-apps", "-r", action="store_true", help="Pull Repeated Apps", default=False)
    parser.add_argument("--loop", type=int, help="Loop Pull Apps", default=1)
    parser.add_argument("--loop-wait", type=str, help="Loop Wait (00h00m00s format)", default="5m")
    parser.add_argument(
        "--keep-open-on-error",
        action="store_true",
        help="Keep AppGallery open when an error occurs",
        default=False,
    )
    parser.add_argument("--all-data-api", type=str)
    parser.add_argument("--all-data-api-key", type=str)