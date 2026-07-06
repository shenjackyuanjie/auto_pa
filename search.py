import argparse
import asyncio
import sys

from src.logger import init_logger


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Collect AppGallery category app names and search them one by one"
    )
    parser.add_argument(
        "--fresh",
        action="store_true",
        help="discard saved progress and start from the beginning",
    )
    parser.add_argument(
        "--random",
        action="store_true",
        help="collect only fresh apps and search them in random order",
    )
    parser.add_argument(
        "--verbose", "-v", action="store_true", help="enable verbose logging"
    )
    parser.add_argument(
        "--disable-log-file",
        "-Dl",
        action="store_true",
        help="disable log file output",
    )
    return parser.parse_args()


if __name__ == "__main__":
    args = parse_args()
    init_logger(not args.disable_log_file, args.verbose)
    from src.logger import logger
    from core.search import main

    logger.info("Starting [search]...")
    logger.info(f"Python version: [{sys.version}]")
    try:
        asyncio.run(main(args))
    except (KeyboardInterrupt, SystemExit):
        logger.info("KeyboardInterrupt")
    except Exception as exc:
        logger.traceback(exc)
        raise SystemExit(1) from exc
    finally:
        logger.info("Exiting...")
