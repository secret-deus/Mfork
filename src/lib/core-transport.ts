import { Channel, invoke } from "@tauri-apps/api/core";

export interface ApiInfo {
  id: string;
  transport: "unix" | "tcp";
  port: number;
  mixed_port: number;
  started_at_ms: number;
}

export interface CoreStatus {
  running: boolean;
  api: ApiInfo | null;
}

function request<T>(api: ApiInfo, method: string, path: string, body?: unknown) {
  return invoke<T>("core_request", { sessionId: api.id, method, path, body: body ?? null });
}

export const apiGet = <T>(api: ApiInfo, path: string) => request<T>(api, "GET", path);
export const apiPatch = <T>(api: ApiInfo, path: string, body: unknown) => request<T>(api, "PATCH", path, body);
export const apiPut = <T>(api: ApiInfo, path: string, body?: unknown) => request<T>(api, "PUT", path, body);
export const apiPost = <T>(api: ApiInfo, path: string, body?: unknown) => request<T>(api, "POST", path, body);
export const apiDelete = <T>(api: ApiInfo, path: string) => request<T>(api, "DELETE", path);

type StreamEvent<T> = { event: "connected" } | { event: "data"; data: T } | { event: "error"; message: string };
type StreamKind = "logs" | "traffic" | "memory" | "connections";
let nextSubscription = 0;

/** Rust owns reconnects; closing during registration must also cancel a late registration. */
export function subscribeCoreStream<T>(api: ApiInfo, kind: StreamKind, onData: (data: T) => void, onError: (message: string) => void): () => void {
  const subscriptionId = `${api.id}:${++nextSubscription}`;
  const channel = new Channel<StreamEvent<T>>();
  let closed = false;
  let reported = false;
  channel.onmessage = (event) => {
    if (closed) return;
    if (event.event === "data") onData(event.data);
    else if (event.event === "connected") reported = false;
    else if (!reported) { reported = true; onError(event.message); }
  };
  const cancel = () => invoke("core_unsubscribe", { subscriptionId }).catch(() => {});
  void invoke("core_subscribe", { sessionId: api.id, subscriptionId, kind, onEvent: channel })
    .then(() => { if (closed) void cancel(); })
    .catch((error) => { if (!closed) onError(String(error)); });
  return () => { closed = true; void cancel(); };
}

/** A failed UI read is not evidence that a service-managed core stopped. */
export async function reconcileCoreSession(attempted: ApiInfo | null): Promise<ApiInfo | null> {
  try {
    const status = await invoke<CoreStatus>("core_status");
    return status.running ? status.api : null;
  } catch {
    return attempted;
  }
}
