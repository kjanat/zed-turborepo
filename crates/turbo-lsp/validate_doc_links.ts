#!/usr/bin/env -S deno run --allow-net --allow-read
/// <reference types="deno" />

type ParsedSource = {
  docLinks: Record<string, string>;
  topLevel: string[];
  taskFields: string[];
  topLevelHovers: Record<string, HoverSpec>;
  taskFieldHovers: Record<string, HoverSpec>;
};

type HoverSpec = {
  docsKey: string;
  context: string;
  example: string;
  summaryOverride?: string;
};

const docLinksFile = new URL("./doc_links.ts", import.meta.url);
const schemaUrl = "https://turborepo.dev/schema.json";

if (import.meta.main) {
  const source = await Deno.readTextFile(docLinksFile);
  const parsed = parseSource(source);
  await validateDocLinks(parsed.docLinks);
  await validateSchemaKeys(parsed.topLevel, parsed.taskFields);
  validateHoverSpecs(parsed);
  console.log(JSON.stringify(parsed, null, 2));
}

export function parseSource(source: string): ParsedSource {
  return {
    docLinks: parseStringMap(source, "DOC_LINKS"),
    topLevel: parseStringArray(source, "topLevel"),
    taskFields: parseStringArray(source, "taskFields"),
    topLevelHovers: parseJsonConst(source, "TOP_LEVEL_HOVERS_JSON"),
    taskFieldHovers: parseJsonConst(source, "TASK_FIELD_HOVERS_JSON"),
  };
}

function parseStringMap(source: string, constName: string): Record<string, string> {
  const block = extractBraceBlock(source, constName);
  const entries: Record<string, string> = {};
  let pendingKey: string | null = null;

  for (const rawLine of block.split("\n")) {
    const line = rawLine.trim();
    if (line.length === 0) continue;

    if (pendingKey !== null) {
      const value = parseQuotedValue(line);
      if (value !== null) {
        entries[pendingKey] = value;
        pendingKey = null;
      }
      continue;
    }

    const separator = line.indexOf(":");
    if (separator === -1) continue;

    const key = line.slice(0, separator).trim();
    const rest = line.slice(separator + 1).trim();
    const value = parseQuotedValue(rest);
    if (value !== null) {
      entries[key] = value;
    } else {
      pendingKey = key;
    }
  }

  if (pendingKey !== null) {
    throw new Error(`unterminated string value for key ${pendingKey}`);
  }

  return entries;
}

function parseStringArray(source: string, arrayName: string): string[] {
  const block = extractBracketBlock(source, arrayName);
  return block
    .split("\n")
    .map((line) => parseQuotedValue(line.trim()))
    .filter((value): value is string => value !== null);
}

function parseJsonConst<T>(source: string, constName: string): T {
  const marker = `export const ${constName} = String.raw\``;
  const start = source.indexOf(marker);
  if (start === -1) {
    throw new Error(`missing ${constName} const`);
  }
  const rest = source.slice(start + marker.length);
  const end = rest.indexOf("`;");
  if (end === -1) {
    throw new Error(`unterminated ${constName} const`);
  }
  return JSON.parse(rest.slice(0, end)) as T;
}

function extractBraceBlock(source: string, constName: string): string {
  const marker = `export const ${constName} = {`;
  const start = source.indexOf(marker);
  if (start === -1) {
    throw new Error(`missing ${constName} block`);
  }
  const rest = source.slice(start + marker.length);
  const end = rest.indexOf("} as const;");
  if (end === -1) {
    throw new Error(`unterminated ${constName} block`);
  }
  return rest.slice(0, end);
}

function extractBracketBlock(source: string, arrayName: string): string {
  const marker = `${arrayName}: [`;
  const start = source.indexOf(marker);
  if (start === -1) {
    throw new Error(`missing ${arrayName} array`);
  }
  const rest = source.slice(start + marker.length);
  const end = rest.indexOf("]");
  if (end === -1) {
    throw new Error(`unterminated ${arrayName} array`);
  }
  return rest.slice(0, end);
}

function parseQuotedValue(input: string): string | null {
  const start = input.indexOf("\"");
  if (start === -1) return null;
  const rest = input.slice(start + 1);
  const end = rest.indexOf("\"");
  if (end === -1) return null;
  return rest.slice(0, end);
}

async function validateDocLinks(docLinks: Record<string, string>): Promise<void> {
  const pageCache = new Map<string, string>();

  for (const [name, rawUrl] of Object.entries(docLinks)) {
    const url = new URL(rawUrl);
    const fragment = url.hash.replace(/^#/, "");
    url.hash = "";
    const baseUrl = url.toString();

    if (!pageCache.has(baseUrl)) {
      const response = await fetch(baseUrl);
      if (!response.ok) {
        throw new Error(`bad doc link ${name}: ${baseUrl} returned ${response.status}`);
      }
      pageCache.set(baseUrl, await response.text());
    }

    if (fragment.length > 0) {
      const body = pageCache.get(baseUrl) ?? "";
      if (!anchorExists(body, fragment)) {
        throw new Error(`bad doc link ${name}: missing anchor #${fragment} in ${baseUrl}`);
      }
    }
  }
}

function anchorExists(body: string, anchor: string): boolean {
  return [
    `[#${anchor}]`,
    `id="${anchor}"`,
    `id='${anchor}'`,
    `href="#${anchor}"`,
    `href='#${anchor}'`,
  ].some((needle) => body.includes(needle));
}

async function validateSchemaKeys(topLevel: string[], taskFields: string[]): Promise<void> {
  const response = await fetch(schemaUrl);
  if (!response.ok) {
    throw new Error(`schema fetch failed: ${response.status}`);
  }

  const schema = await response.json();
  const topLevelProperties = schema.properties as Record<string, unknown> | undefined;
  if (!topLevelProperties) {
    throw new Error("schema missing top-level properties");
  }

  for (const key of topLevel) {
    if (key === "pipeline") continue;
    if (!(key in topLevelProperties)) {
      throw new Error(`schema missing top-level key ${key}`);
    }
  }

  const pipelineProperties = schema.definitions?.Pipeline?.properties as
    | Record<string, unknown>
    | undefined;
  if (!pipelineProperties) {
    throw new Error("schema missing Pipeline.properties");
  }

  for (const key of taskFields) {
    if (!(key in pipelineProperties)) {
      throw new Error(`schema missing task field ${key}`);
    }
  }
}

function validateHoverSpecs(parsed: ParsedSource): void {
  for (const key of parsed.topLevel) {
    if (!(key in parsed.topLevelHovers)) {
      throw new Error(`missing top-level hover spec ${key}`);
    }
  }

  for (const key of parsed.taskFields) {
    if (!(key in parsed.taskFieldHovers)) {
      throw new Error(`missing task-field hover spec ${key}`);
    }
  }

  for (const [key, spec] of Object.entries(parsed.topLevelHovers)) {
    if (!(spec.docsKey in parsed.docLinks)) {
      throw new Error(`top-level hover ${key} references unknown docsKey ${spec.docsKey}`);
    }
  }

  for (const [key, spec] of Object.entries(parsed.taskFieldHovers)) {
    if (!(spec.docsKey in parsed.docLinks)) {
      throw new Error(`task-field hover ${key} references unknown docsKey ${spec.docsKey}`);
    }
  }
}
