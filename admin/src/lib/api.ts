import { assertSafeNumbers } from "./data.mjs";

export class ApiError extends Error {
  constructor(public status: number, retryAfter: string | null) {
    const retry = retryAfter && /^\d{1,6}$/.test(retryAfter) ? ` ${retryAfter}초 뒤 다시 시도해 주세요.` : " 잠시 후 다시 시도해 주세요.";
    super(status === 401 ? "세션이 만료되었습니다. 다시 로그인해 주세요."
      : status === 403 ? "요청 권한을 확인할 수 없습니다. 다시 로그인한 뒤 시도해 주세요."
      : status === 429 ? `요청이 너무 많습니다.${retry}`
      : status === 503 ? "서비스를 일시적으로 사용할 수 없습니다. 잠시 후 다시 시도해 주세요."
      : status === 404 ? "해당 키를 찾을 수 없습니다. 목록을 새로고침해 주세요."
      : status === 400 ? "입력 형식이나 조회 조건을 확인해 주세요."
      : "요청을 완료하지 못했습니다. 잠시 후 다시 시도해 주세요.");
  }
}

export function errorMessage(error: unknown) {
  return error instanceof SyntaxError ? "서버 응답을 읽을 수 없습니다. 잠시 후 다시 시도해 주세요."
    : error instanceof TypeError ? "서버에 연결하지 못했습니다. 연결 상태를 확인하고 다시 시도해 주세요."
    : error instanceof Error ? error.message : "요청을 완료하지 못했습니다.";
}

export class AdminClient {
  private requests = new Set<AbortController>();
  constructor(private unauthorized: () => void) {}

  cancelAll() {
    for (const controller of this.requests) controller.abort();
    this.requests.clear();
  }

  async request<T>(path: string, options: { method?: string; csrf?: string; body?: unknown; signal?: AbortSignal } = {}): Promise<T> {
    const controller = new AbortController();
    const abort = () => controller.abort();
    options.signal?.addEventListener("abort", abort, { once: true });
    if (options.signal?.aborted) controller.abort();
    this.requests.add(controller);
    try {
      const headers = new Headers({ Accept: "application/json" });
      if (options.csrf) headers.set("X-CSRF-Token", options.csrf);
      if (options.body !== undefined) headers.set("Content-Type", "application/json");
      // Origin은 브라우저가 설정한다. Bearer·스토리지·URL에는 세션/비밀키를 넣지 않는다.
      const response = await fetch(path, {
        method: options.method ?? "GET", credentials: "same-origin", cache: "no-store",
        headers, body: options.body === undefined ? undefined : JSON.stringify(options.body),
        signal: controller.signal, redirect: "error"
      });
      controller.signal.throwIfAborted();
      if (response.status === 401) {
        this.cancelAll();
        this.unauthorized();
        throw new ApiError(401, null);
      }
      if (!response.ok) throw new ApiError(response.status, response.headers.get("Retry-After"));
      if (response.status === 204) return undefined as T;
      const result: unknown = await response.json();
      controller.signal.throwIfAborted();
      assertSafeNumbers(result);
      return result as T;
    } finally {
      options.signal?.removeEventListener("abort", abort);
      this.requests.delete(controller);
    }
  }
}
