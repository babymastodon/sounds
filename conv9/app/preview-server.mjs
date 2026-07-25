import { createReadStream } from "node:fs";
import { stat } from "node:fs/promises";
import { createServer } from "node:http";
import { dirname, extname, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";

const appDir = dirname(fileURLToPath(import.meta.url));
const conv9Dir = resolve(appDir, "..");

const contentTypes = new Map([
  [".css", "text/css; charset=utf-8"],
  [".html", "text/html; charset=utf-8"],
  [".js", "text/javascript; charset=utf-8"],
  [".json", "application/json; charset=utf-8"],
  [".wav", "audio/wav"],
]);

export async function startPreviewServer({
  host = "127.0.0.1",
  port = 4173,
  quiet = false,
} = {}) {
  const server = createServer((request, response) => {
    serve(request, response).catch((error) => {
      if (!response.headersSent) {
        response.writeHead(error.code === "ENOENT" ? 404 : 500, {
          "Content-Type": "text/plain; charset=utf-8",
        });
      }
      response.end(error.code === "ENOENT" ? "not found\n" : `${error.message}\n`);
    });
  });
  await new Promise((resolveListen, rejectListen) => {
    server.once("error", rejectListen);
    server.listen(port, host, () => {
      server.off("error", rejectListen);
      resolveListen();
    });
  });
  if (!quiet) {
    const address = server.address();
    const boundPort = typeof address === "object" && address ? address.port : port;
    console.log(`conv9 preview: http://${host}:${boundPort}/app/src/`);
  }
  return server;
}

async function serve(request, response) {
  if (request.method !== "GET" && request.method !== "HEAD") {
    response.writeHead(405, { Allow: "GET, HEAD" });
    response.end();
    return;
  }
  const requestUrl = new URL(request.url || "/", "http://localhost");
  let relativePath = decodeURIComponent(requestUrl.pathname).replace(/^\/+/, "");
  if (!relativePath || relativePath.endsWith("/")) relativePath += "index.html";
  const filePath = resolve(conv9Dir, relativePath);
  if (filePath !== conv9Dir && !filePath.startsWith(`${conv9Dir}${sep}`)) {
    response.writeHead(403, { "Content-Type": "text/plain; charset=utf-8" });
    response.end("forbidden\n");
    return;
  }

  const metadata = await stat(filePath);
  if (!metadata.isFile()) {
    response.writeHead(404, { "Content-Type": "text/plain; charset=utf-8" });
    response.end("not found\n");
    return;
  }

  const headers = {
    "Accept-Ranges": "bytes",
    "Cache-Control": "no-store",
    "Content-Type": contentTypes.get(extname(filePath)) || "application/octet-stream",
  };
  const range = parseRange(request.headers.range, metadata.size);
  if (range === null && request.headers.range) {
    response.writeHead(416, {
      ...headers,
      "Content-Range": `bytes */${metadata.size}`,
    });
    response.end();
    return;
  }

  const start = range?.start ?? 0;
  const end = range?.end ?? metadata.size - 1;
  const contentLength = Math.max(0, end - start + 1);
  if (range) {
    response.writeHead(206, {
      ...headers,
      "Content-Length": contentLength,
      "Content-Range": `bytes ${start}-${end}/${metadata.size}`,
    });
  } else {
    response.writeHead(200, {
      ...headers,
      "Content-Length": metadata.size,
    });
  }
  if (request.method === "HEAD" || metadata.size === 0) {
    response.end();
    return;
  }
  createReadStream(filePath, { start, end }).pipe(response);
}

function parseRange(header, size) {
  if (!header) return undefined;
  const match = /^bytes=(\d*)-(\d*)$/.exec(header.trim());
  if (!match || (!match[1] && !match[2]) || size <= 0) return null;

  let start;
  let end;
  if (!match[1]) {
    const suffixLength = Number(match[2]);
    if (!Number.isSafeInteger(suffixLength) || suffixLength <= 0) return null;
    start = Math.max(0, size - suffixLength);
    end = size - 1;
  } else {
    start = Number(match[1]);
    end = match[2] ? Number(match[2]) : size - 1;
    if (
      !Number.isSafeInteger(start) ||
      !Number.isSafeInteger(end) ||
      start < 0 ||
      start >= size ||
      end < start
    ) {
      return null;
    }
    end = Math.min(end, size - 1);
  }
  return { start, end };
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  await startPreviewServer({
    host: process.env.CONV9_PREVIEW_HOST || "127.0.0.1",
    port: Number(process.env.CONV9_PREVIEW_PORT || 4173),
  });
}
