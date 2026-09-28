import { useEffect, useRef, useState } from "react";
import { useT } from "../i18n";
import type { LonLat } from "../lib/geo";
import { createGlobeScene, type GlobeScene, type GlobeStatus } from "./globe/scene";
import "./globe/globe.css";

export interface GlobeNode {
  name: string;
  pos: LonLat;
  count: number;
}

export interface GlobeRoute {
  key: string;
  /** Hops after this machine, in travel order (relay → … → exit). */
  hops: { name: string; pos: LonLat }[];
  /** Live connections on this route (0 = selected but idle). */
  conns: number;
  tone: "accent" | "ok";
}

export interface GlobeProps {
  self: LonLat;
  nodes: GlobeNode[];
  routes: GlobeRoute[];
  active: boolean;
}

export default function Globe(props: GlobeProps) {
  const t = useT();
  const host = useRef<HTMLDivElement>(null);
  const canvas = useRef<HTMLCanvasElement | null>(null);
  const scene = useRef<GlobeScene | null>(null);
  const latest = useRef(props);
  latest.current = props;
  const label = useRef("");
  label.current = t("三维地球：拖拽或方向键旋转，Home 键重置视角");
  const [status, setStatus] = useState<GlobeStatus>("ready");

  useEffect(() => {
    if (!host.current) return;
    let mounted = true;
    // Each effect owns a fresh canvas. A disposed context's asynchronous loss event
    // must never poison the next StrictMode/HMR mount on the same DOM element.
    const element = document.createElement("canvas");
    element.className = "globe-canvas block h-full w-full cursor-grab touch-none active:cursor-grabbing";
    element.tabIndex = 0;
    element.setAttribute("role", "img");
    element.setAttribute("aria-label", label.current);
    host.current.appendChild(element);
    canvas.current = element;
    try {
      const view = createGlobeScene(element, (next) => { if (mounted) setStatus(next); });
      scene.current = view;
      view.update(latest.current);
      setStatus("ready");
    } catch (error) {
      console.warn("Unable to initialize overview globe", error);
      scene.current?.dispose();
      scene.current = null;
      setStatus("unavailable");
    }
    return () => {
      mounted = false;
      element.remove();
      scene.current?.dispose();
      scene.current = null;
      canvas.current = null;
    };
  }, []);

  useEffect(() => { scene.current?.update(props); }, [props.self, props.nodes, props.routes, props.active]);
  useEffect(() => {
    if (!canvas.current) return;
    canvas.current.setAttribute("aria-label", label.current);
    canvas.current.tabIndex = status === "ready" ? 0 : -1;
    canvas.current.setAttribute("aria-hidden", String(status === "unavailable"));
  }, [t, status]);

  return (
    <div className="overview-globe relative h-full w-full">
      <div ref={host} className="h-full w-full" />
      {status === "ready" ? (
        <button className="globe-reset" onClick={() => scene.current?.reset()} title={t("重置地球视角")} aria-label={t("重置地球视角")}>
          <span aria-hidden="true">↺</span>
        </button>
      ) : (
        <div className="globe-fallback" role="status">
          {status === "lost" ? t("地球渲染已暂停，等待图形设备恢复") : t("此设备暂不支持三维地球，网络功能不受影响")}
        </div>
      )}
    </div>
  );
}
