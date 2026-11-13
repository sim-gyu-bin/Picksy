import { useEffect, useRef, useState, type FormEvent } from "react";
import { Button } from "./ui/button";
import {
  AlertDialog, AlertDialogCancel, AlertDialogContent, AlertDialogDescription,
  AlertDialogFooter, AlertDialogHeader, AlertDialogTitle, AlertDialogTrigger
} from "./ui/alert-dialog";
import { Loading, Notice, timestamp, useKeys } from "./common";
import { errorMessage, type AdminClient } from "../lib/api";
import { count, RequestScope } from "../lib/data.mjs";
import type { CreatedKey, KeyInfo, Session } from "../lib/types";

function RevokeKey({ client, session, item, disabled, changed }: {
  client: AdminClient; session: Session; item: KeyInfo; disabled: boolean; changed: () => void;
}) {
  const [open, setOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const locked = useRef(false);
  const scope = useRef<RequestScope | null>(null);
  useEffect(() => () => scope.current?.cancel(), []);
  async function revoke() {
    if (locked.current) return;
    locked.current = true;
    setBusy(true);
    setError("");
    const request = new RequestScope();
    scope.current = request;
    try {
      await client.request<void>(`/admin/keys/${encodeURIComponent(item.id)}`, {
        method: "DELETE", csrf: session.csrf_token, signal: request.signal
      });
      if (request.active) {
        setOpen(false);
        changed();
      }
    } catch (reason) {
      if (request.active) setError(errorMessage(reason));
    } finally {
      if (request.active) { locked.current = false; setBusy(false); }
    }
  }
  return <AlertDialog open={open} onOpenChange={(next) => { if (!locked.current) { setOpen(next); setError(""); } }}>
    <AlertDialogTrigger asChild><Button variant="ghost" size="sm" disabled={disabled} className="text-destructive">폐기</Button></AlertDialogTrigger>
    <AlertDialogContent onEscapeKeyDown={(event) => { if (busy) event.preventDefault(); }}>
      <AlertDialogHeader><AlertDialogTitle>이 API 키를 폐기할까요?</AlertDialogTitle><AlertDialogDescription>
        폐기한 키는 복구할 수 없으며 이후 인증에 사용할 수 없습니다. 이미 접수된 요청은 완료될 수 있고, 사용량과 감사 이력은 보존됩니다.
      </AlertDialogDescription></AlertDialogHeader>
      <div className="revoke-target"><strong>{item.display_name}</strong><code>{item.id}</code></div>
      {error && <Notice message={error} />}
      <AlertDialogFooter>
        <AlertDialogCancel disabled={busy}>취소</AlertDialogCancel>
        <Button variant="destructive" disabled={busy} onClick={() => { void revoke(); }}>{busy ? "폐기하는 중…" : "이 키 폐기"}</Button>
      </AlertDialogFooter>
    </AlertDialogContent>
  </AlertDialog>;
}

export function Keys({ client, session, onUsage }: { client: AdminClient; session: Session; onUsage: (keyId: string) => void }) {
  const [revision, setRevision] = useState(0);
  const keyList = useKeys(client, revision);
  const [name, setName] = useState("");
  const [limit, setLimit] = useState("60");
  const [creating, setCreating] = useState(false);
  const [error, setError] = useState("");
  const [secret, setSecret] = useState<CreatedKey | null>(null);
  const [copyMessage, setCopyMessage] = useState("");
  const locked = useRef(false);
  const request = useRef<RequestScope | null>(null);
  const secretPanel = useRef<HTMLElement>(null);
  const formName = useRef<HTMLInputElement>(null);
  const secretVersion = useRef(0);
  useEffect(() => () => { request.current?.cancel(); secretVersion.current++; }, []);
  useEffect(() => { if (secret) secretPanel.current?.focus(); }, [secret]);

  function dismissSecret() {
    // 복사 완료 콜백도 닫힌 비밀키 화면을 다시 활성화할 수 없다.
    secretVersion.current++;
    setSecret(null);
    setCopyMessage("");
    formName.current?.focus();
  }

  async function create(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (locked.current || secret) return;
    setError("");
    const trimmed = name.trim();
    if (Array.from(trimmed).length > 128 || /[\u0000-\u001f\u007f-\u009f]/.test(trimmed)) {
      setError("키 이름은 제어문자 없이 128자 이내로 입력해 주세요."); return;
    }
    const rpm = Number(limit);
    if (!/^\d+$/.test(limit) || !Number.isSafeInteger(rpm) || rpm < 1) {
      setError("분당 한도는 화면에서 정확히 표현할 수 있는 양의 정수로 입력해 주세요."); return;
    }
    locked.current = true;
    setCreating(true);
    const scope = new RequestScope();
    request.current = scope;
    try {
      const created = await client.request<CreatedKey>("/admin/keys", {
        method: "POST", csrf: session.csrf_token, body: { ...(trimmed ? { name: trimmed } : {}), requests_per_minute: rpm }, signal: scope.signal
      });
      if (scope.active) {
        secretVersion.current++;
        setSecret(created);
        setCopyMessage("");
        setName("");
        setRevision((value) => value + 1);
      }
    } catch (reason) {
      if (scope.active) setError(`${errorMessage(reason)} 응답을 받지 못한 경우 키가 이미 생성되었을 수 있으니, 다시 만들기 전에 목록을 확인해 주세요.`);
    } finally {
      if (scope.active) { locked.current = false; setCreating(false); }
    }
  }

  async function copySecret() {
    if (!secret) return;
    const version = secretVersion.current;
    try {
      if (!navigator.clipboard?.writeText) throw new Error("clipboard_unavailable");
      await navigator.clipboard.writeText(secret.api_key);
      if (version === secretVersion.current) setCopyMessage("복사했습니다. 클립보드에 남은 키도 안전하게 관리해 주세요.");
    } catch {
      if (version === secretVersion.current) setCopyMessage("클립보드에 접근할 수 없습니다. 아래 원문을 직접 선택해 복사해 주세요.");
    }
  }

  return <>
    <header className="page-heading"><div><p className="eyebrow">ACCESS</p><h1>API 키</h1><p>연결할 서비스에 키를 발급하고 접근을 관리하세요.</p></div><Button variant="outline" onClick={keyList.reload}>새로고침</Button></header>
    {secret && <section ref={secretPanel} tabIndex={-1} className="panel secret-panel" aria-labelledby="secret-heading">
      <div className="section-heading"><h2 id="secret-heading">지금 한 번만 확인할 수 있는 키입니다</h2></div>
      <p>안전한 비밀 저장소에 보관하세요. 이 화면을 닫거나 이동·로그아웃하면 원문을 다시 볼 수 없습니다.</p>
      <p className="hint">{secret.display_name} · {secret.id}</p>
      <label className="field">생성된 원문 API 키<textarea className="secret-value" readOnly value={secret.api_key} rows={3} autoComplete="off" spellCheck={false} /></label>
      <div className="action-row"><Button onClick={() => { void copySecret(); }}>키 복사</Button><Button variant="outline" onClick={dismissSecret}>보관 완료 · 원문 닫기</Button></div>
      <p className="hint" role="status">{copyMessage || "원문은 이 화면의 메모리에만 유지되며 별도로 저장하지 않습니다."}</p>
    </section>}
    <section className="panel" aria-labelledby="create-heading"><div className="section-heading"><div><h2 id="create-heading">새 키 만들기</h2><p>이름을 비워 두면 UUID로 구분합니다.</p></div></div>
      <form onSubmit={(event) => { void create(event); }} className="create-form">
        <label className="field grow">키 이름 <span className="optional">선택 사항 · 최대 128자</span><input ref={formName} value={name} onChange={(event) => setName(event.target.value)} autoComplete="off" disabled={creating || secret !== null} /></label>
        <label className="field">분당 요청 한도<input type="number" min="1" step="1" max={Number.MAX_SAFE_INTEGER} value={limit} onChange={(event) => setLimit(event.target.value)} required disabled={creating || secret !== null} /></label>
        <Button type="submit" disabled={creating || secret !== null}>{creating ? "만드는 중…" : "API 키 만들기"}</Button>
      </form>
      {secret && <p className="hint">원문을 보관하고 닫은 뒤 다음 키를 만들 수 있습니다.</p>}
      {error && <Notice message={error} />}
    </section>
    <section className="panel" aria-labelledby="keys-heading"><div className="section-heading"><div><h2 id="keys-heading">발급한 키</h2><p>시각은 UTC 기준입니다. 원문 키는 목록에 표시되지 않습니다.</p></div></div>
      {keyList.error ? <Notice message={keyList.error} retry={keyList.reload} /> : !keyList.keys ? <Loading>키 목록을 불러오는 중입니다…</Loading> : keyList.keys.length === 0 ? <div className="empty"><strong>아직 발급한 키가 없습니다.</strong><p>위에서 첫 API 키를 만들어 서비스를 연결하세요.</p></div> : <div className="table-scroll"><table className="keys-table"><caption className="sr-only">API 키의 이름, UUID, 생성과 폐기 시각, 한도, 상태</caption>
        <thead><tr><th>이름 / UUID</th><th>상태</th><th>분당 한도</th><th>생성 시각 (UTC)</th><th>폐기 시각 (UTC)</th><th>관리</th></tr></thead>
        <tbody>{keyList.keys.map((item) => <tr key={item.id}><th scope="row"><span className="key-name">{item.name ?? "이름 없음"}</span><code className="key-id">{item.id}</code></th><td><span className={`badge ${item.revoked_at === null ? "active-badge" : ""}`}>{item.revoked_at === null ? "활성" : "폐기됨"}</span></td><td>{count(item.requests_per_minute)}회</td><td className="date-cell">{timestamp(item.created_at)}</td><td className="date-cell">{item.revoked_at === null ? "—" : timestamp(item.revoked_at)}</td><td><div className="row-actions"><Button variant="ghost" size="sm" onClick={() => onUsage(item.id)}>사용량</Button>{item.revoked_at === null && <RevokeKey client={client} session={session} item={item} disabled={creating} changed={() => { dismissSecret(); setRevision((value) => value + 1); }} />}</div></td></tr>)}</tbody>
      </table></div>}
      <p className="hint">키별 요청 수는 사용량 화면에서 조회할 수 있습니다. 폐기된 키의 이력도 조회 가능합니다.</p>
    </section>
  </>;
}
