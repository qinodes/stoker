export class ApiError extends Error {
  readonly status: number;
  readonly code: string;
  readonly details: unknown;

  constructor(input: { status: number; code?: string; message?: string; details?: unknown }) {
    super(input.message || `Request failed (${input.status})`);
    this.name = "ApiError";
    this.status = input.status;
    this.code = input.code || "internal";
    this.details = input.details ?? null;
  }
}

export interface ApiClientOptions {
  fetchImpl?: typeof fetch;
}

export interface ApiClient {
  request<T>(path: string, options?: RequestInit): Promise<T>;
  get<T>(path: string): Promise<T>;
  send<T>(path: string, method: string, body?: unknown): Promise<T>;
}

export function createApiClient({
  fetchImpl = globalThis.fetch.bind(globalThis),
}: ApiClientOptions = {}): ApiClient {
  async function request<T>(path: string, options: RequestInit = {}): Promise<T> {
    const headers = new Headers(options.headers);
    if (options.body !== undefined && !headers.has("Content-Type")) headers.set("Content-Type", "application/json");
    const response = await fetchImpl(path, { ...options, headers, cache: "no-store" });
    if (response.ok) {
      return (response.status === 204 ? null : await response.json()) as T;
    }
    let payload: { code?: string; message?: string; error?: string; details?: unknown } | null;
    try {
      payload = (await response.json()) as typeof payload;
    } catch {
      payload = null;
    }
    const error = new ApiError({
      status: response.status,
      code: payload?.code,
      message: payload?.message || payload?.error || `Request failed (${response.status})`,
      details: payload?.details,
    });
    throw error;
  }

  return {
    request,
    get: <T>(path: string) => request<T>(path),
    send: <T>(path: string, method: string, body: unknown = null) => request<T>(path, {
      method,
      body: body === null ? undefined : JSON.stringify(body),
    }),
  };
}
