import argparse
import asyncio

import anyio
from .appgallery import main as gallery_main, exit_main as gallery_exit, argument as gallery_argument
from src.logger import logger

def create_main_parser():
    """创建主参数解析器"""
    main_parser = argparse.ArgumentParser(
        description='A simple CLI tool to interact with Harmony Command With Line',
        formatter_class=argparse.RawDescriptionHelpFormatter
    )
    
    # 添加子命令
    subparsers = main_parser.add_subparsers(
        title='targets',
        description='available targets',
        dest='target',
        help='choose a target to run',
        required=True
    )
    
    # 为 gallery 添加子命令解析器
    gallery_parser = subparsers.add_parser(
        'gallery',
        help='app gallery related operations',
        parents=[gallery_argument],
        add_help=False  # 避免重复的 --help
    )
    
    # 添加 gallery 专用的参数（如果需要）
    gallery_parser.set_defaults(func=gallery_main, exit=gallery_exit)
    
    return main_parser, subparsers

def cli_main():
    anyio.run(main)

async def main():
    """CLI 主入口函数"""
    parser, _ = create_main_parser()
    args = parser.parse_args()
    
    if hasattr(args, 'func'):
        try:
            await args.func(args)
        except (KeyboardInterrupt, asyncio.CancelledError):
            logger.warning("操作被用户中断")
        except Exception:
            logger.traceback("错误")

        finally:
            if hasattr(args, 'exit'):
                await args.exit()

    else:
        parser.print_help()