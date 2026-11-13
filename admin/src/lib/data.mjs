const numberFormat = new Intl.NumberFormat("ko-KR");

/** @param {number | null} value */
export function count(value) {
  if (value === null) return "미수집";
  if (!Number.isSafeInteger(value) || value < 0) throw new Error("수치가 화면에서 정확히 표시할 수 있는 범위를 벗어났습니다.");
  return numberFormat.format(value);
}

/** JSON 파싱으로 손실된 i64를 정확한 통계처럼 보여 주지 않는다. @param {unknown} value */
export function assertSafeNumbers(value) {
  if (typeof value === "number" && (!Number.isSafeInteger(value) || value < 0)) {
    throw new Error("수치가 화면에서 정확히 표시할 수 있는 범위를 벗어났습니다. 원본 정수 조회가 필요합니다.");
  }
  if (value !== null && typeof value === "object") {
    for (const entry of Object.values(value)) assertSafeNumbers(entry);
  }
}

/** @param {string} day */
function utcDay(day) {
  if (!/^\d{4}-\d{2}-\d{2}$/.test(day)) throw new Error("시작일과 종료일을 입력해 주세요.");
  const time = Date.parse(`${day}T00:00:00Z`);
  if (!Number.isFinite(time) || time < 0 || new Date(time).toISOString().slice(0, 10) !== day) {
    throw new Error("1970년 이후의 올바른 날짜를 입력해 주세요.");
  }
  return time / 1000;
}

/** 화면의 종료일을 포함하도록 서버의 반열린 UTC 구간으로 변환한다.
 * @param {{start: string, end: string, keyId: string, kind: string}} filters
 */
export function usageQuery(filters) {
  const from = utcDay(filters.start);
  const until = utcDay(filters.end) + 86400;
  if (from >= until) throw new Error("종료일은 시작일보다 빠를 수 없습니다.");
  const query = new URLSearchParams({ from: String(from), until: String(until) });
  if (filters.keyId) query.set("key_id", filters.keyId);
  if (filters.kind) query.set("kind", filters.kind);
  return query.toString();
}

export function initialDates() {
  const today = new Date();
  const end = today.toISOString().slice(0, 10);
  const start = new Date(Date.UTC(today.getUTCFullYear(), today.getUTCMonth(), today.getUTCDate() - 29)).toISOString().slice(0, 10);
  return { start, end };
}

/** 이전 응답은 취소가 늦게 반영되어도 화면에 반영할 수 없다. */
export class RequestScope {
  controller = new AbortController();
  cancel() { this.controller.abort(); }
  get signal() { return this.controller.signal; }
  get active() { return !this.controller.signal.aborted; }
}
