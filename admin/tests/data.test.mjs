import test from "node:test";
import assert from "node:assert/strict";
import { assertSafeNumbers, count, RequestScope, usageQuery } from "../src/lib/data.mjs";

test("UTC 종료일 포함은 다음 날 자정의 배타적 until로 전달한다", () => {
  const query = new URLSearchParams(usageQuery({ start: "2024-02-29", end: "2024-02-29", keyId: "", kind: "" }));
  assert.equal(query.get("from"), String(Date.parse("2024-02-29T00:00:00Z") / 1000));
  assert.equal(query.get("until"), String(Date.parse("2024-03-01T00:00:00Z") / 1000));
  assert.equal(query.has("kind"), false);
  assert.equal(query.has("key_id"), false);
});

test("일광 절약 시간 전환에도 UTC 날짜 간격과 필터를 보존한다", () => {
  const query = new URLSearchParams(usageQuery({ start: "2024-03-10", end: "2024-03-10", keyId: "selected-id", kind: "unknown" }));
  assert.equal(Number(query.get("until")) - Number(query.get("from")), 86400);
  assert.equal(query.get("kind"), "unknown");
  assert.equal(query.get("key_id"), "selected-id");
});

test("존재하지 않는 날짜와 음수 시각과 역전된 기간은 요청 전에 거절한다", () => {
  for (const [start, end] of [["2023-02-29", "2023-03-01"], ["1969-12-31", "1970-01-01"], ["2024-03-02", "2024-03-01"], ["", "2024-01-01"]]) {
    assert.throws(() => usageQuery({ start, end, keyId: "", kind: "" }));
  }
});

test("미수집 null과 실제 0을 구별하고 안전 정수 밖의 i64는 반올림 표시하지 않는다", () => {
  assert.equal(count(null), "미수집");
  assert.equal(count(0), "0");
  assert.doesNotThrow(() => assertSafeNumbers({ summary: { input_tokens: null, output_tokens: 0 }, by_day: [] }));
  assert.throws(() => assertSafeNumbers(JSON.parse('{"summary":{"total":9007199254740993}}')));
  assert.throws(() => count(Number.MAX_SAFE_INTEGER + 1));
});

test("취소 전에 시작한 느린 응답이 새 조건의 응답을 덮어쓰지 않는다", async () => {
  const oldRequest = new RequestScope();
  const newRequest = new RequestScope();
  let rendered = null;
  let finishOld;
  const oldResponse = new Promise((resolve) => { finishOld = resolve; }).then((value) => {
    if (oldRequest.active) rendered = value;
  });
  oldRequest.cancel();
  if (newRequest.active) rendered = "새 조건";
  finishOld("이전 조건");
  await oldResponse;
  assert.equal(rendered, "새 조건");
  assert.equal(oldRequest.signal.aborted, true);
  newRequest.cancel();
  assert.equal(newRequest.active, false);
});
