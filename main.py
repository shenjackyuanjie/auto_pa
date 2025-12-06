import anyio
import core

async def main():
    await core.cli_main()
    

if __name__ == "__main__":
    anyio.run(main)
