import { useEffect, useRef, useState, type MouseEvent } from "react";
import { createRoot } from "react-dom/client";
import { flushSync } from "react-dom";
import { Button } from "./components/ui/button";
import { Icon, Loading, Notice } from "./components/common";
import { Keys } from "./components/keys";
import { Usage } from "./components/usage";
import { AdminClient, errorMessage } from "./lib/api";
import { RequestScope } from "./lib/data.mjs";
import type { Session } from "./lib/types";

type Page = "usage" | "keys";

function App() {
  const [session, setSession] = useState<Session | null>(null);
  const [loading, setLoading] = useState(true);
  const [sessionError, setSessionError] = useState("");
  const [sessionRevision, setSessionRevision] = useState(0);
  const [loggingOut, setLoggingOut] = useState(false);
  const [logoutError, setLogoutError] = useState("");
  const [page, setPage] = useState<Page>(window.location.pathname === "/console/keys" ? "keys" : "usage");
  const [selectedKey, setSelectedKey] = useState("");
  const main = useRef<HTMLElement>(null);
  const logoutLocked = useRef(false);
  const [client] = useState(() => new AdminClient(() => {
    setSession(null);
    setSelectedKey("");
    setSessionError("세션이 만료되었습니다. 다시 로그인해 주세요.");
  }));
  useEffect(() => {
    const scope = new RequestScope();
    setLoading(true);
    setSessionError("");
    void client.request<Session>("/admin/session", { signal: scope.signal }).then((result) => {
      if (scope.active) setSession(result);
    }).catch((reason: unknown) => {
      if (scope.active) { setSession(null); setSessionError(errorMessage(reason)); }
    }).finally(() => { if (scope.active) setLoading(false); });
    return () => scope.cancel();
  }, [client, sessionRevision]);

  useEffect(() => {
    const onPopState = () => {
      setSelectedKey("");
      setPage(window.location.pathname === "/console/keys" ? "keys" : "usage");
    };
    const onPageHide = () => {
      client.cancelAll();
      // bfcache에도 원문 키·세션·통계가 남지 않도록 즉시 비공개 하위 화면을 해제한다.
      flushSync(() => { setSession(null); setSelectedKey(""); setLogoutError(""); });
    };
    const onPageShow = (event: PageTransitionEvent) => {
      if (event.persisted) setSessionRevision((value) => value + 1);
    };
    window.addEventListener("popstate", onPopState);
    window.addEventListener("pagehide", onPageHide);
    window.addEventListener("pageshow", onPageShow);
    return () => {
      window.removeEventListener("popstate", onPopState);
      window.removeEventListener("pagehide", onPageHide);
      window.removeEventListener("pageshow", onPageShow);
      client.cancelAll();
    };
  }, [client]);

  useEffect(() => {
    if (!session) return;
    const expire = () => {
      client.cancelAll();
      setSession(null);
      setSelectedKey("");
      setSessionError("세션이 만료되었습니다. 다시 로그인해 주세요.");
    };
    const remaining = session.expires_at * 1000 - Date.now();
    const timer = window.setTimeout(expire, Math.max(0, remaining));
    return () => window.clearTimeout(timer);
  }, [client, session]);

  useEffect(() => {
    document.title = `${page === "keys" ? "API 키" : "사용량"} · Picksy 관리`;
    if (session) main.current?.focus();
  }, [page, session]);

  function navigate(next: Page, keyId = "") {
    const path = next === "keys" ? "/console/keys" : "/console";
    if (window.location.pathname !== path) window.history.pushState(null, "", path);
    setSelectedKey(keyId);
    setPage(next);
    setLogoutError("");
  }

  function navigateLink(event: MouseEvent<HTMLAnchorElement>, next: Page) {
    if (event.button !== 0 || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) return;
    event.preventDefault();
    navigate(next);
  }

  async function logout() {
    if (!session || logoutLocked.current) return;
    logoutLocked.current = true;
    flushSync(() => { setLoggingOut(true); setLogoutError(""); });
    client.cancelAll();
    try {
      await client.request<void>("/admin/logout", { method: "POST", csrf: session.csrf_token });
      setSession(null);
      setSelectedKey("");
      setSessionError("");
    } catch (reason) {
      if (!(reason instanceof DOMException && reason.name === "AbortError")) setLogoutError(errorMessage(reason));
    } finally {
      logoutLocked.current = false;
      setLoggingOut(false);
    }
  }

  if (loading) return <main className="login-page"><div className="login-card"><div className="brand">picksy<span>관리 콘솔</span></div><Loading>로그인 상태를 확인하고 있습니다…</Loading></div></main>;
  if (!session) return <main className="login-page"><div className="login-card"><div className="brand">picksy<span>관리 콘솔</span></div><div className="login-copy"><p className="eyebrow">WELCOME BACK</p><h1>차분하게, 한눈에.</h1><p>API 키와 사용량을 안전하게 관리하세요.<br />허용된 Google 계정으로 로그인할 수 있습니다.</p></div>
    {sessionError && <Notice message={sessionError} retry={() => setSessionRevision((value) => value + 1)} />}
    <Button asChild size="lg" className="w-full"><a href="/auth/google/login">Google 계정으로 로그인</a></Button><p className="hint">관리자 로그인은 추론 API 키 인증과 별도로 보호됩니다.</p>
  </div></main>;

  return <div className="app-shell">
    <a className="skip-link" href="#main-content">본문으로 건너뛰기</a>
    <aside className="sidebar"><a href="/console" className="brand" onClick={(event) => navigateLink(event, "usage")}>picksy<span>관리 콘솔</span></a>
      <nav aria-label="관리 메뉴"><a href="/console" aria-current={page === "usage" ? "page" : undefined} onClick={(event) => navigateLink(event, "usage")}><Icon name="usage" />사용량</a><a href="/console/keys" aria-current={page === "keys" ? "page" : undefined} onClick={(event) => navigateLink(event, "keys")}><Icon name="keys" />API 키</a></nav>
      <div className="sidebar-bottom"><p><span className="session-dot" aria-hidden="true" />관리자 세션</p><Button variant="ghost" disabled={loggingOut} onClick={() => { void logout(); }}><Icon name="logout" />{loggingOut ? "로그아웃 중…" : "로그아웃"}</Button></div>
    </aside>
    <main id="main-content" ref={main} tabIndex={-1} className="main-content">
      {logoutError && <Notice message={logoutError} />}
      {loggingOut ? <Loading>비공개 자료를 지우고 로그아웃하는 중입니다…</Loading> : page === "keys" ? <Keys client={client} session={session} onUsage={(keyId) => navigate("usage", keyId)} /> : <Usage key={selectedKey} client={client} selectedKey={selectedKey} />}
      <footer className="page-footer">Picksy <span>관리 콘솔 · 모든 집계 시각은 UTC 기준</span></footer>
    </main>
  </div>;
}

const root = document.getElementById("root");
if (!root) throw new Error("관리 화면을 시작할 요소가 없습니다.");
createRoot(root).render(<App />);
