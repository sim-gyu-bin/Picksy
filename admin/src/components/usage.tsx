import { useEffect, useId, useState } from "react";
import { Button } from "./ui/button";
import { Loading, Notice, useKeys } from "./common";
import { errorMessage, type AdminClient } from "../lib/api";
import { count, initialDates, RequestScope, usageQuery } from "../lib/data.mjs";
import type { RequestKind, UsageReport } from "../lib/types";

const kinds: Record<RequestKind, string> = {
  inference: "추론", models: "모델 목록", auth_check: "인증 확인", admin: "관리 분류", unknown: "알 수 없음"
};

function DailyChart({ days }: { days: UsageReport["by_day"] }) {
  const titleId = useId();
  const descriptionId = useId();
  if (days.length === 0) return <div className="empty">선택한 기간에 기록된 요청이 없습니다.</div>;
  const maximum = days.reduce((max, item) => Math.max(max, item.summary.total), 0);
  const plotWidth = 820;
  const step = plotWidth / days.length;
  return <>
    <div className="chart-scroll">
      <svg className="usage-chart" viewBox="0 0 900 240" role="img" aria-labelledby={`${titleId} ${descriptionId}`}>
        <title id={titleId}>UTC 일별 요청 수</title>
        <desc id={descriptionId}>실제 기록이 있는 날짜만 표시합니다. 날짜 간격은 동일하며 누락된 날짜를 0으로 채우지 않습니다. 정확한 수치는 아래 일별 상세 표에서 확인할 수 있습니다.</desc>
        <line x1="60" y1="190" x2="880" y2="190" className="chart-axis" />
        <line x1="60" y1="30" x2="880" y2="30" className="chart-grid" />
        <text x="50" y="35" textAnchor="end" className="chart-label">{count(maximum)}</text>
        <text x="50" y="194" textAnchor="end" className="chart-label">0</text>
        {days.map(({ day, summary }, index) => {
          const height = maximum === 0 ? 0 : summary.total / maximum * 160;
          const x = 60 + index * step;
          return <g key={day}>
            <rect x={x + step * .18} y={190 - height} width={step * .64} height={height} rx={Math.min(4, step * .12)} className="chart-bar"><title>{day}: {count(summary.total)}회</title></rect>
            {(days.length <= 7 || index === 0 || index === days.length - 1) && <text x={x + step / 2} y="216" textAnchor="middle" className="chart-label">{day.slice(5)}</text>}
          </g>;
        })}
      </svg>
    </div>
    <details className="daily-details"><summary>일별 상세 수치 보기</summary>
      <div className="table-scroll"><table><caption className="sr-only">실제 기록이 있는 날짜별 요청과 추론 토큰</caption><thead><tr><th>날짜 (UTC)</th><th>전체 요청</th><th>성공</th><th>거절</th><th>실패</th><th>미완료</th><th>기타 완료</th><th>입력 토큰</th><th>출력 토큰</th></tr></thead><tbody>
        {days.map(({ day, summary }) => <tr key={day}><th scope="row">{day}</th><td>{count(summary.total)}</td><td>{count(summary.succeeded)}</td><td>{count(summary.rejected)}</td><td>{count(summary.failed)}</td><td>{count(summary.pending)}</td><td>{count(summary.other_completed)}</td><td>{count(summary.input_tokens)}</td><td>{count(summary.output_tokens)}</td></tr>)}
      </tbody></table></div>
    </details>
  </>;
}

export function Usage({ client, selectedKey }: { client: AdminClient; selectedKey: string }) {
  const [filters, setFilters] = useState(() => ({ ...initialDates(), keyId: selectedKey, kind: "" }));
  const [result, setResult] = useState<{ query: string; revision: number; report: UsageReport } | null>(null);
  const [error, setError] = useState("");
  const [revision, setRevision] = useState(0);
  const keyList = useKeys(client);
  let query = "";
  let dateError = "";
  try { query = usageQuery(filters); } catch (reason) { dateError = errorMessage(reason); }

  useEffect(() => {
    const scope = new RequestScope();
    setResult(null);
    setError("");
    if (query) {
      void client.request<UsageReport>(`/admin/usage?${query}`, { signal: scope.signal }).then((data) => {
        if (scope.active) setResult({ query, revision, report: data });
      }).catch((reason: unknown) => {
        if (scope.active) setError(errorMessage(reason));
      });
    }
    return () => scope.cancel();
  }, [client, query, revision]);

  // 새 필터의 첫 렌더에서도 이전 필터의 수치를 잠깐 보여 주지 않는다.
  const report = result?.query === query && result.revision === revision ? result.report : null;
  const summary = report?.summary;
  return <>
    <header className="page-heading"><div><p className="eyebrow">OVERVIEW</p><h1>사용량</h1><p>요청 흐름과 실제 수집된 토큰을 확인하세요.</p></div><Button variant="outline" onClick={() => setRevision((value) => value + 1)}>새로고침</Button></header>
    <section className="panel filter-panel" aria-labelledby="filter-heading"><h2 id="filter-heading" className="sr-only">사용량 조회 조건</h2>
      <div className="filter-grid">
        <label className="field">시작일 (UTC)<input type="date" min="1970-01-01" value={filters.start} onChange={(event) => setFilters({ ...filters, start: event.target.value })} /></label>
        <label className="field">종료일 (UTC, 당일 포함)<input type="date" min="1970-01-01" value={filters.end} onChange={(event) => setFilters({ ...filters, end: event.target.value })} /></label>
        <label className="field">API 키<select value={filters.keyId} onChange={(event) => setFilters({ ...filters, keyId: event.target.value })} disabled={!keyList.keys}><option value="">모든 키</option>
          {filters.keyId && !keyList.keys?.some((key) => key.id === filters.keyId) && <option value={filters.keyId}>선택한 키 · {filters.keyId}</option>}
          {keyList.keys?.map((key) => <option key={key.id} value={key.id}>{key.display_name}{key.name ? ` · ${key.id}` : ""}{key.revoked_at !== null ? " (폐기됨)" : ""}</option>)}
        </select></label>
        <label className="field">요청 종류<select value={filters.kind} onChange={(event) => setFilters({ ...filters, kind: event.target.value })}><option value="">모든 종류</option>{Object.entries(kinds).map(([kind, label]) => <option key={kind} value={kind}>{label}</option>)}</select></label>
      </div>
      <p className="hint">UTC 기준으로 시작일 00:00부터 종료일 다음 날 00:00 직전까지 조회합니다. 조건을 바꾸면 자동으로 갱신됩니다.</p>
      {keyList.error && <Notice message={keyList.error} retry={keyList.reload} />}
    </section>
    {dateError ? <Notice message={dateError} /> : error ? <Notice message={error} retry={() => setRevision((value) => value + 1)} /> : !summary || !report ? <Loading>사용량을 불러오는 중입니다…</Loading> : <div aria-live="polite">
      <section className="metric-grid" aria-label="사용량 요약">
        <article className="metric"><p>전체 요청</p><strong>{count(summary.total)}<small>회</small></strong><span>선택한 조건의 인증된 요청</span></article>
        <article className="metric"><p>성공한 요청</p><strong>{count(summary.succeeded)}<small>회</small></strong><span>정상 완료된 요청</span></article>
        <article className="metric"><p>입력 토큰</p><strong>{count(summary.input_tokens)}</strong><span>수집된 추론 토큰 합계</span></article>
        <article className="metric"><p>출력 토큰</p><strong>{count(summary.output_tokens)}</strong><span>수집된 추론 토큰 합계</span></article>
      </section>
      <section className="panel" aria-labelledby="daily-heading"><div className="section-heading"><div><h2 id="daily-heading">일별 요청</h2><p>UTC · 실제 기록이 있는 날짜만 표시</p></div><span className="badge">요청 수</span></div><DailyChart days={report.by_day} /></section>
      <div className="detail-grid">
        <section className="panel" aria-labelledby="status-heading"><div className="section-heading"><h2 id="status-heading">처리 상태</h2></div><dl className="stat-list"><div><dt>성공</dt><dd>{count(summary.succeeded)}</dd></div><div><dt>거절</dt><dd>{count(summary.rejected)}</dd></div><div><dt>실패</dt><dd>{count(summary.failed)}</dd></div><div><dt>미완료</dt><dd>{count(summary.pending)}</dd></div><div><dt>기타 완료</dt><dd>{count(summary.other_completed)}</dd></div></dl></section>
        <section className="panel" aria-labelledby="token-heading"><div className="section-heading"><h2 id="token-heading">토큰 수집 현황</h2></div><dl className="stat-list"><div><dt>토큰을 수집한 추론</dt><dd>{count(summary.token_recorded)}회</dd></div><div><dt>토큰 미수집 추론</dt><dd>{count(summary.token_unknown)}회</dd></div></dl><p className="hint">미수집은 0과 다릅니다. 토큰 합계는 수집된 값만 반영하며, 과거의 알 수 없는 요청을 추론으로 추정하지 않습니다.</p></section>
      </div>
      <section className="panel" aria-labelledby="kind-heading"><div className="section-heading"><h2 id="kind-heading">요청 종류별</h2></div>{report.by_kind.length === 0 ? <div className="empty">기록된 요청 종류가 없습니다.</div> : <div className="table-scroll"><table><caption className="sr-only">종류별 요청 수와 수집된 추론 토큰</caption><thead><tr><th>종류</th><th>요청 수</th><th>입력 토큰</th><th>출력 토큰</th></tr></thead><tbody>{report.by_kind.map(({ kind, summary: item }) => <tr key={kind}><th scope="row">{kinds[kind]}</th><td>{count(item.total)}</td><td>{count(item.input_tokens)}</td><td>{count(item.output_tokens)}</td></tr>)}</tbody></table></div>}</section>
    </div>}
  </>;
}
