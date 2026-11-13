import { useEffect, useState } from "react";
import { Button } from "./ui/button";
import { errorMessage, type AdminClient } from "../lib/api";
import { RequestScope } from "../lib/data.mjs";
import type { KeyInfo } from "../lib/types";

export function Notice({ message, retry }: { message: string; retry?: () => void }) {
  return <div className="notice" role="alert"><p>{message}</p>{retry && <Button variant="outline" size="sm" onClick={retry}>다시 시도</Button>}</div>;
}

export function Loading({ children = "불러오는 중입니다…" }: { children?: React.ReactNode }) {
  return <div className="loading" role="status"><span className="loading-dot" aria-hidden="true" />{children}</div>;
}

export function useKeys(client: AdminClient, revision = 0) {
  const [keys, setKeys] = useState<KeyInfo[] | null>(null);
  const [error, setError] = useState("");
  const [retry, setRetry] = useState(0);
  useEffect(() => {
    const scope = new RequestScope();
    setKeys(null);
    setError("");
    void client.request<KeyInfo[]>("/admin/keys", { signal: scope.signal }).then((data) => {
      if (scope.active) setKeys(data);
    }).catch((reason: unknown) => {
      if (scope.active) setError(errorMessage(reason));
    });
    return () => scope.cancel();
  }, [client, revision, retry]);
  return { keys, error, reload: () => setRetry((value) => value + 1) };
}

export function timestamp(value: number) {
  return new Intl.DateTimeFormat("ko-KR", {
    timeZone: "UTC", year: "numeric", month: "2-digit", day: "2-digit",
    hour: "2-digit", minute: "2-digit", second: "2-digit", hourCycle: "h23"
  }).format(new Date(value * 1000));
}

export function Icon({ name }: { name: "usage" | "keys" | "logout" }) {
  const paths = {
    usage: "M4 19h16M7 15V9m5 6V5m5 10v-4",
    keys: "M14 4a6 6 0 1 1-4 10l-6 6H1v-3l6-6a6 6 0 0 1 7-7Zm2 4h.01",
    logout: "M9 4H4v16h5m5-12 4 4-4 4m-6-4h12"
  };
  return <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true"><path d={paths[name]} /></svg>;
}
