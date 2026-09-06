// Overview globe: orthographic land silhouettes on a canvas, marking only
// reachable proxy nodes, with a great-circle "light streak" from the local
// machine to the node currently in use. Slowly drifts; theme-aware.
import { useEffect, useRef } from "react";
import {
  geoDistance,
  geoInterpolate,
  geoOrthographic,
  geoPath,
  type GeoPermissibleObjects,
} from "d3-geo";
import { feature } from "topojson-client";
import type { Topology, GeometryCollection } from "topojson-specification";
import landTopo from "world-atlas/land-110m.json";
import { onThemeChange } from "../theme";
import type { LonLat } from "../lib/geo";

export interface GlobeNode {
  name: string;
  pos: LonLat;
  count: number;
}

export interface GlobeProps {
  self: LonLat;
  nodes: GlobeNode[];
  /** node currently in use (drawn highlighted with the streak) */
  current: GlobeNode | null;
  active: boolean;
}

const topo = landTopo as unknown as Topology<{ land: GeometryCollection }>;
const LAND = feature(topo, topo.objects.land) as GeoPermissibleObjects;

function cssVar(name: string): string {
  return getComputedStyle(document.documentElement).getPropertyValue(name).trim();
}

function withAlpha(color: string, a: number): string {
  // palette entries are #rrggbb
  const m = /^#([0-9a-f]{6})$/i.exec(color);
  if (!m) return color;
  const n = parseInt(m[1], 16);
  return `rgba(${(n >> 16) & 255}, ${(n >> 8) & 255}, ${n & 255}, ${a})`;
}

export default function Globe({ self, nodes, current, active }: GlobeProps) {
  const ref = useRef<HTMLCanvasElement>(null);
  const props = useRef({ self, nodes, current, active });
  props.current = { self, nodes, current, active };

  useEffect(() => {
    const canvas = ref.current;
    if (!canvas) return;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;

    let palette = readPalette();
    const unTheme = onThemeChange(() => {
      palette = readPalette();
    });

    let w = 0;
    let h = 0;
    const resize = () => {
      const dpr = window.devicePixelRatio || 1;
      const r = canvas.getBoundingClientRect();
      w = r.width;
      h = r.height;
      canvas.width = Math.round(w * dpr);
      canvas.height = Math.round(h * dpr);
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    };
    resize();
    const ro = new ResizeObserver(resize);
    ro.observe(canvas);

    const projection = geoOrthographic().clipAngle(90);
    const path = geoPath(projection, ctx);
    // start looking at the local machine, then drift east slowly
    let drift = 0;
    let last = performance.now();
    let raf = 0;

    const frame = (now: number) => {
      const dt = Math.min(100, now - last);
      last = now;
      const p = props.current;
      if (p.active) drift += dt * 0.0025; // ~9°/s → one turn per 2.4 min
      const radius = Math.min(w, h) / 2 - 1;
      projection
        .translate([w / 2, h / 2])
        .scale(radius)
        .rotate([-p.self[0] - 25 + drift, -22]);
      const center: LonLat = [p.self[0] + 25 - drift, 22];
      const visible = (pt: LonLat) => geoDistance(pt, center) < Math.PI / 2 - 0.02;

      ctx.clearRect(0, 0, w, h);

      // sphere + land
      ctx.beginPath();
      path({ type: "Sphere" });
      ctx.fillStyle = palette.sphere;
      ctx.fill();
      ctx.lineWidth = 1;
      ctx.strokeStyle = palette.outline;
      ctx.stroke();

      ctx.beginPath();
      path(LAND);
      ctx.fillStyle = palette.land;
      ctx.fill();
      ctx.lineWidth = 0.6;
      ctx.strokeStyle = palette.landEdge;
      ctx.stroke();

      // reachable nodes
      for (const n of p.nodes) {
        if (!visible(n.pos)) continue;
        const xy = projection(n.pos);
        if (!xy) continue;
        const isCur = p.current && p.current.name === n.name;
        if (isCur) continue;
        ctx.beginPath();
        ctx.arc(xy[0], xy[1], n.count > 3 ? 2.6 : 2, 0, Math.PI * 2);
        ctx.fillStyle = palette.node;
        ctx.fill();
      }

      // local machine
      if (visible(p.self)) {
        const xy = projection(p.self);
        if (xy) {
          ctx.beginPath();
          ctx.arc(xy[0], xy[1], 3, 0, Math.PI * 2);
          ctx.fillStyle = palette.ok;
          ctx.fill();
          ctx.beginPath();
          ctx.arc(xy[0], xy[1], 7, 0, Math.PI * 2);
          ctx.strokeStyle = withAlpha(palette.ok, 0.35);
          ctx.lineWidth = 1;
          ctx.stroke();
        }
      }

      // current node + streak
      if (p.current && p.active) {
        const cur = p.current;
        const interp = geoInterpolate(p.self, cur.pos);
        const line = {
          type: "LineString" as const,
          coordinates: Array.from({ length: 65 }, (_, i) => interp(i / 64)),
        };
        ctx.beginPath();
        path(line);
        ctx.lineWidth = 1;
        ctx.strokeStyle = withAlpha(palette.accent, 0.28);
        ctx.stroke();

        // moving bright segment
        const t = (now / 1800) % 1;
        const seg = {
          type: "LineString" as const,
          coordinates: Array.from({ length: 13 }, (_, i) =>
            interp(Math.max(0, Math.min(1, t - 0.16 + (i / 12) * 0.16))),
          ),
        };
        ctx.beginPath();
        path(seg);
        ctx.lineWidth = 1.6;
        ctx.strokeStyle = palette.accent;
        ctx.lineCap = "round";
        ctx.stroke();

        if (visible(cur.pos)) {
          const xy = projection(cur.pos);
          if (xy) {
            const pulse = 0.5 + 0.5 * Math.sin(now / 400);
            ctx.beginPath();
            ctx.arc(xy[0], xy[1], 6 + pulse * 4, 0, Math.PI * 2);
            ctx.strokeStyle = withAlpha(palette.accent, 0.25 + 0.2 * (1 - pulse));
            ctx.lineWidth = 1;
            ctx.stroke();
            ctx.beginPath();
            ctx.arc(xy[0], xy[1], 3.2, 0, Math.PI * 2);
            ctx.fillStyle = palette.accent;
            ctx.fill();
          }
        }
      }

      raf = requestAnimationFrame(frame);
    };
    raf = requestAnimationFrame(frame);

    return () => {
      cancelAnimationFrame(raf);
      ro.disconnect();
      unTheme();
    };
  }, []);

  return <canvas ref={ref} className="block h-full w-full" />;
}

function readPalette() {
  const bg = cssVar("--t-bg");
  return {
    sphere: cssVar("--t-surface") || bg,
    outline: cssVar("--t-line-strong"),
    land: cssVar("--t-line"),
    landEdge: cssVar("--t-line-strong"),
    node: cssVar("--t-mut"),
    ok: cssVar("--t-ok"),
    accent: cssVar("--t-accent"),
  };
}
