import { build, context } from "esbuild";
import { copyFile, mkdir, readFile } from "node:fs/promises";
import { watch as watchFile } from "node:fs";
import { spawn } from "node:child_process";
import { createRequire } from "node:module";
import { fileURLToPath } from "node:url";

process.chdir(fileURLToPath(new URL("..", import.meta.url)));
const watching = process.argv.includes("--watch");
const require = createRequire(import.meta.url);
await mkdir("dist/assets", { recursive: true });
await copyFile("index.html", "dist/index.html");
const options = {
  entryPoints: ["src/main.tsx"], outfile: "dist/assets/app.js", bundle: true,
  format: "esm", platform: "browser", target: ["es2022"],
  minify: !watching, sourcemap: false, legalComments: "eof",
  banner: { js: `/* ${await readFile("LICENSE.shadcn", "utf8")} */` },
  define: { "process.env.NODE_ENV": JSON.stringify(watching ? "development" : "production") }
};
// 별도 개발 서버 없이 정해진 세 파일만 덮어쓰고 Rust 서버에서 새로고침한다.
const css = spawn(process.execPath, [require.resolve("tailwindcss/lib/cli.js"),
  "-i", "src/styles.css", "-o", "dist/assets/app.css", ...(watching ? ["--watch=always"] : ["--minify"])],
  { stdio: "inherit" });
if (watching) {
  const bundler = await context(options);
  await bundler.watch();
  const html = watchFile("index.html", () => {
    copyFile("index.html", "dist/index.html").catch(() => {
      console.error("HTML 자산을 갱신하지 못했습니다.");
      process.exitCode = 1;
    });
  });
  let stopping = false;
  const stop = async (code) => {
    if (stopping) return;
    stopping = true;
    html.close();
    css.kill("SIGTERM");
    await bundler.dispose();
    process.exitCode = code;
  };
  process.once("SIGINT", () => { void stop(0); });
  process.once("SIGTERM", () => { void stop(0); });
  css.once("error", () => { void stop(1); });
  css.once("exit", (code) => { if (!stopping) void stop(code ?? 1); });
} else {
  const cssDone = new Promise((resolve, reject) => {
    css.once("error", reject);
    css.once("exit", (code) => code === 0 ? resolve() : reject(new Error("Tailwind 자산 생성 실패")));
  });
  await Promise.all([build(options), cssDone]);
}
