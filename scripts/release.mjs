import { cli } from "./release/cli.mjs";
await cli(process.argv[2], process.argv.slice(3));
