#!/usr/bin/env bun
/** @see {@link https://github.com/vercel/turborepo/blob/c8a10835ee7165099cceeb43d06a627d6c3f70da/crates/turborepo-updater/src/lib.rs#L72-L73|turborepo-updater/src/lib.rs} - vercel/turborepo */
const version = ["latest", "canary"] as const;
const endpoint = "https://turborepo.dev/api/binaries/version";

interface VersionRequest {
  name: string;
  tag: typeof version[number];
}
interface VersionResponse {
  name: string;
  version: string;
  tag: typeof version[number];
}

const config: VersionRequest = {
  name: "turbo",
  tag: "latest",
};

async function getLatestVersion(ep: string = endpoint, c: VersionRequest): Promise<VersionResponse> {
  const res: Promise<Response> = Bun.fetch(`${ep}?name=${c.name}&tag=${c.tag}`);
  return (await res).json();
}

getLatestVersion(endpoint, config).then((res: VersionResponse): void => {
  console.log(`Latest ${res.name} version (${res.tag}): ${res.version}`);
});
