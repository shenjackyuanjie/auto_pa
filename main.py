import argparse
import asyncio
import sys
from graceful_shutdown import ShutdownProtection

from src.logger import init_logger
from core import appgallery_add_argument, hilog_add_argument

main_parser = argparse.ArgumentParser()
# verbose
# log_file
main_parser.add_argument(
    "--verbose", "-v", action="store_true", help="verbose mode", default=False
)
main_parser.add_argument(
    "--disable-log-file", "-Dl", action="store_true", help="disable log file", default=False
)
main_parser.add_argument(
    'target',
    choices=['appgallery', 'hilog']
)

if __name__ == "__main__":
    args = main_parser.parse_known_args()[0]
    init_logger(not args.disable_log_file, args.verbose)
    target = args.target
    from src.logger import logger
    logger.info(f"Starting [{target}]...")
    logger.info(f"Python version: [{sys.version}]")
    main = None
    if target == 'appgallery':
        appgallery_add_argument(main_parser)
        from core.appgallery import main
    elif target == 'hilog':
        hilog_add_argument(main_parser)
        from core.hilog import main

    args = main_parser.parse_args()

    if main is None:
        logger.error(f"Unknown target: {target}")
        exit(1)

    with ShutdownProtection(1) as s:
        try:
            loop = asyncio.get_event_loop()
            loop.run_until_complete(main(args))
            loop.close()
        except (KeyboardInterrupt, SystemExit):
            logger.info("KeyboardInterrupt")
        except Exception as e:
            logger.traceback(e)
        except:  # noqa: E722
            logger.traceback()
        finally:
            logger.info("Exiting...")
