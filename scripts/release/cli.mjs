import { context } from "./source.mjs";
import {
  desktopBuild,
  releaseBuild,
  releaseClean,
  releaseStage,
  releaseVerify,
} from "./workflow.mjs";

export async function cli(command, args = process.argv.slice(2)) {
  try {
    const allowed =
      command === "clean"
        ? ["--apply", "--legacy", "--recover", "--stale-lock"]
        : [];
    if (args.some((arg) => !allowed.includes(arg)))
      throw new Error(
        `未知参数：${args.join(" ")}。测试fixture没有生产CLI入口。`,
      );
    const ctx = context();
    let result;
    switch (command) {
      case "build":
        result = await releaseBuild(ctx);
        break;
      case "desktop":
        result = await desktopBuild(ctx);
        break;
      case "stage":
        result = await releaseStage(ctx);
        break;
      case "verify":
        result = await releaseVerify(ctx);
        break;
      case "clean":
        result = await releaseClean(ctx, {
          apply: args.includes("--apply"),
          legacy: args.includes("--legacy"),
          recover: args.includes("--recover"),
          staleLock: args.includes("--stale-lock"),
        });
        break;
      default:
        throw new Error(`未知发布命令：${command}`);
    }
    console.log(JSON.stringify(result, null, 2));
  } catch (e) {
    console.error(`FluxDL发布流程失败：${e.message}`);
    process.exitCode = 1;
  }
}
