import logging
import sys
import traceback
from loguru import logger as Logger

LOGGER_FORMAT = "<green>[{time:YYYY-MM-DD HH:mm:ss}]</green> <level>[{level}] <yellow>[{name}:{function}:{line}]</yellow>: {message}</level>"


_logger = Logger.opt(depth=2)
_logger.remove()
# if LOG_FILE:
#     _logger.add(
#         "./logs/{time:YYYY-MM-DD}.log",
#         format=LOGGER_FORMAT,
#         retention="90 days",
#         encoding="utf-8",
#     )
# _logger.add(
#     sys.stdout,
#     format=LOGGER_FORMAT,
#     level="DEBUG" if DEBUG else "INFO",
#     colorize=True,
# )

class Loglogger:
    def __init__(self, log = _logger, log_file: bool = False, verbose: bool = False) -> None:
        self.log = log
        if log_file:
            _logger.add(
                "./logs/{time:YYYY-MM-DD}.log",
                format=LOGGER_FORMAT,
                retention="90 days",
                encoding="utf-8",
            )
        _logger.add(
            sys.stdout,
            format=LOGGER_FORMAT,
            level="DEBUG" if verbose else "INFO",
            colorize=True,
        )
    def raw_log(self, level, message: str, *values):
        self.log.log(level, message % values)
    def _log_with_args(self, level, *args, **kwargs):
        message = _log(*args) if args else ""
        self.log.log(level, message, **kwargs)
    
    def info(self, *args, **kwargs):
        self._log_with_args("INFO", *args, **kwargs)
    def error(self, *args, **kwargs):
        self._log_with_args("ERROR", *args, **kwargs)
    def debug(self, *args, **kwargs):
        self._log_with_args("DEBUG", *args, **kwargs)
    def warning(self, *args, **kwargs):
        self._log_with_args("WARNING", *args, **kwargs)
    def success(self, *args, **kwargs):
        self._log_with_args("SUCCESS", *args, **kwargs)
    def traceback(self, *args, **kwargs):
        if args or kwargs:
            self._log_with_args("ERROR", *args, **kwargs)
        error = traceback.format_exc()
        self.log.error(error)
    def debug_traceback(self, *args, **kwargs):
        if args or kwargs:
            self._log_with_args("DEBUG", *args, **kwargs)
        error = traceback.format_exc()
        self.log.debug(error)

logger: 'Loglogger' = None # type: ignore


def init_logger(
    log_file: bool = False, verbose: bool = False
):
    global logger
    print(log_file, verbose)
    logger = Loglogger(log_file=log_file, verbose=verbose)


def _log(*values):
    data = []
    for v in values:
        try:
            data.append(str(v))
        except Exception:
            data.append(repr(v))
    return " ".join(data)


# uvicorn

class InterceptHandler(logging.Handler):
    def emit(self, record):
        try:
            level = Logger.level(record.levelname).name
        except ValueError:
            level = record.levelno

        logger.log.opt(depth=6).log(
            level,
            record.getMessage(),
        )

class DebugHandler(logging.Handler):
    def emit(self, record):
        logger.log.opt(depth=6).log(
            Logger.level('DEBUG').no,
            record.getMessage(),
        )




__all__ = ["logger"]