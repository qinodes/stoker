export class ApiError extends Error {
  constructor({ status, code, message, details = null }) {
    super(message || `Request failed (${status})`);
    this.name = "ApiError";
    this.status = status;
    this.code = code || "internal";
    this.details = details;
  }
}

export function createApiClient({
  fetchImpl = globalThis.fetch,
  session = globalThis.sessionStorage,
  browserLocation = globalThis.location,
  browserHistory = globalThis.history,
  onUnauthorized = () => {},
} = {}) {
  function token() {
    const hash = new URLSearchParams(browserLocation?.hash?.slice(1) || "");
    const fragmentToken = hash.get("token");
    if (fragmentToken) {
      session?.setItem("stoker-ui-token", fragmentToken);
      browserHistory?.replaceState(
        null,
        "",
        `${browserLocation.pathname}${browserLocation.search}#overview`,
      );
    }
    return session?.getItem("stoker-ui-token") || "";
  }

  async function request(path, options = {}) {
    const accessToken = token();
    const headers = { ...(options.headers || {}) };
    if (accessToken) headers.Authorization = `Bearer ${accessToken}`;
    if (options.body !== undefined && !headers["Content-Type"]) {
      headers["Content-Type"] = "application/json";
    }
    const response = await fetchImpl(path, { ...options, headers, cache: "no-store" });
    if (response.ok) {
      return response.status === 204 ? null : response.json();
    }
    let payload = null;
    try {
      payload = await response.json();
    } catch {
      payload = null;
    }
    const error = new ApiError({
      status: response.status,
      code: payload?.code,
      message: payload?.message || payload?.error || `Request failed (${response.status})`,
      details: payload?.details,
    });
    if (error.code === "unauthorized") onUnauthorized(error);
    throw error;
  }

  return {
    request,
    get: (path) => request(path),
    send: (path, method, body = null) => request(path, {
      method,
      body: body === null ? undefined : JSON.stringify(body),
    }),
  };
}
