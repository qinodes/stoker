import type { PageInfo, Route, WorkspaceState } from "./types";
import { createScheduledState } from "./scheduled/state.ts";

export const JOBS_PAGE_SIZE = 5;
export const SNAPSHOTS_PAGE_SIZE = 5;
export const DIRECTORY_CACHE_TTL_MS = 20_000;
export const DIRECTORY_CACHE_LIMIT = 64;

export interface RequestSequence {
  next: () => number;
  isCurrent: (request: number) => boolean;
}

export function createRequestSequence(): RequestSequence {
  let latest = 0;
  return {
    next: () => ++latest,
    isCurrent: (request: number) => request === latest,
  };
}

export interface DirectoryCacheEntry {
  value: WorkspaceState["filesystem"]["current"];
  storedAt: number;
}

export function createState(route: Route = "overview"): WorkspaceState {
  return {
    mode: null,
    modeTransition: null,
    workspace: null,
    config: null,
    settings: null,
    policy: null,
    status: null,
    timezone: null,
    jobs: [],
    queue: { jobs: [], locked: false },
    logs: { jobId: "", stream: "stdout", data: null, error: null, search: "" },
    route,
    filters: { search: "", user: "", state: "" },
    pagination: { jobs: 1, snapshots: 1 },
    loaded: false,
    loading: false,
    error: null,
    jobDraft: { user: "", name: "", cwd: "", command: "", description: "" },
    filesystem: { roots: null, current: null, inputPath: "", loading: false, error: "" },
    selectedJob: null,
    selectedJobDetail: null,
    configurationDraft: null,
    scheduled: createScheduledState(),
  };
}

export function pageInfo<T>(items: T[], requestedPage: number, pageSize: number): PageInfo<T> {
  const totalPages = Math.max(1, Math.ceil(items.length / pageSize));
  const page = Math.min(Math.max(Number(requestedPage) || 1, 1), totalPages);
  const start = (page - 1) * pageSize;
  return { page, totalPages, totalItems: items.length, start, end: Math.min(start + pageSize, items.length), items: items.slice(start, start + pageSize) };
}

export function cachedDirectory(cache: Map<string, DirectoryCacheEntry>, path: string, now = Date.now()) {
  const entry = cache.get(path);
  if (!entry || now - entry.storedAt >= DIRECTORY_CACHE_TTL_MS) return null;
  cache.delete(path);
  cache.set(path, entry);
  return entry.value;
}

export function cacheDirectory(cache: Map<string, DirectoryCacheEntry>, path: string, value: WorkspaceState["filesystem"]["current"], now = Date.now()) {
  cache.delete(path);
  cache.set(path, { value, storedAt: now });
  while (cache.size > DIRECTORY_CACHE_LIMIT) cache.delete(cache.keys().next().value!);
}
