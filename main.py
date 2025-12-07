import asyncio
import anyio
import core
from src.logger import logger

async def main():
    await core.cli_main()
    

if __name__ == "__main__":
    try:
        anyio.run(main)
    except (KeyboardInterrupt, asyncio.CancelledError):
        logger.info("Exiting...")
    except Exception:
        logger.traceback("An error occurred while running the program")

