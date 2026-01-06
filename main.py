import argparse
import asyncio
from graceful_shutdown import ShutdownProtection

from src.logger import init_logger
from core import add_argument

main_parser = argparse.ArgumentParser()
# verbose
# log_file
main_parser.add_argument(
    "--verbose", "-v", action="store_true", help="verbose mode", default=False
)
main_parser.add_argument(
    "--disable-log-file", "-Dl", action="store_true", help="disable log file", default=False
)

add_argument(main_parser)

if __name__ == "__main__":
    args = main_parser.parse_args()
    init_logger(not args.disable_log_file, args.verbose)

    from src.logger import logger
    from core.appgallery import main

    with ShutdownProtection(1) as s:
        try:
            asyncio.run(main(args))
        except (KeyboardInterrupt, SystemExit):
            logger.info("KeyboardInterrupt")
        except Exception as e:
            logger.traceback(e)
        finally:
            logger.info("Exiting...")
