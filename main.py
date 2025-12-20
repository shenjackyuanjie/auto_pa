import argparse
import asyncio
from src.logger import init_logger

main_parser = argparse.ArgumentParser()
# verbose
# log_file
main_parser.add_argument("--verbose", "-v", action="store_true", help="verbose mode", default=False)
main_parser.add_argument("--log-file", "-l", action="store_true", help="enable log file", default=False)

if __name__ == "__main__":
    args = main_parser.parse_known_args()[0]
    init_logger(args.log_file, args.verbose)

    from src.logger import logger
    from core import main

    try:
        asyncio.run(main())
    except KeyboardInterrupt:
        logger.info("KeyboardInterrupt")
    except Exception as e:
        logger.traceback(e)
    finally:
        logger.info("Exiting...")
