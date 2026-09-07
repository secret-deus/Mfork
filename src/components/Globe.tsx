// Overview globe: orthographic land silhouettes on a canvas, marking only
// reachable proxy nodes, with routes drawn as arcs lifted above the surface
// from the local machine through every hop (relay chains included) to the
// exit node. Sways slowly around the local machine; theme-aware.
import { useEffect, useRef } from "react";
import {
  geoDistance,
  geoInterpolate,
  geoOrthographic,
  geoPath,
  geoRotation,
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

export interface GlobeRoute {
  key: string;
  /** hops after the local machine, in travel order (relay → … → exit) */
  hops: { name: string; pos: LonLat }[];
  /** live connections on this route (0 = selected but idle) */
  conns: number;
  tone: "accent" | "ok";
}

export interface GlobeProps {
  self: LonLat;
  nodes: GlobeNode[];
  routes: GlobeRoute[];
  active: boolean;
}

const topo = landTopo as unknown as Topology<{ land: GeometryCollection }>;
const LAND = feature(topo, topo.objects.land) as GeoPermissibleObjects;

function cssVar(name: string): string {
  return getComputedStyle(document.documentElement).getPropertyValue(name).trim();
}

function withAlpha(color: string, a: number): string {
  const m = /^#([0-9a-f]{6})$/i.exec(color);
  if (!m) return color;
  const n = parseInt(m[1], 16);
  return `rgba(${(n >> 16) & 255}, ${(n >> 8) & 255}, ${n & 255}, ${a})`;
}

/** A point on a lifted arc: screen position plus depth (z > 0 faces us). */
interface ArcPt {
  x: number;
  y: number;
  z: number;
}

export default function Globe({ self, nodes, routes, active }: GlobeProps) {
  const ref = useRef<HTMLCanvasElement>(null);
  const props = useRef({ self, nodes, routes, active });
  props.current = { self, nodes, routes, active };

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
    // stay centred on the local machine with a slow ±14° sway so the
    // nodes never rotate out of view
    let phase = 0;
    let last = performance.now();
    let raf = 0;

    const frame = (now: number) => {
      const dt = Math.min(100, now - last);
      last = now;
      const p = props.current;
      if (p.active) phase += dt * 0.00012; // one full sway ≈ 50 s
      const sway = Math.sin(phase) * 14;
      const radius = Math.min(w, h) / 2 - 1;
      const cx = w / 2;
      const cy = h / 2;
      projection.translate([cx, cy]).scale(radius).rotate([-p.self[0] - 25 + sway, -22]);
      const center: LonLat = [p.self[0] + 25 - sway, 22];
      const visible = (pt: LonLat) => geoDistance(pt, center) < Math.PI / 2 - 0.02;
      const rotate = geoRotation(projection.rotate());

      // lifted arc between two places: great-circle path pushed away from
      // the globe centre by up to `lift` radii at its midpoint
      const arc = (a: LonLat, b: LonLat, n = 48): ArcPt[] => {
        const interp = geoInterpolate(a, b);
        const dist = geoDistance(a, b);
        const lift = 0.06 + 0.28 * (dist / Math.PI);
        const out: ArcPt[] = [];
        for (let i = 0; i <= n; i++) {
          const t = i / n;
          const [lon, lat] = rotate(interp(t));
          const lam = (lon * Math.PI) / 180;
          const phi = (lat * Math.PI) / 180;
          const x = Math.cos(phi) * Math.sin(lam);
          const y = Math.sin(phi);
          const z = Math.cos(phi) * Math.cos(lam);
          const k = radius * (1 + lift * Math.sin(Math.PI * t));
          out.push({ x: cx + k * x, y: cy - k * y, z });
        }
        return out;
      };

      const strokePts = (pts: ArcPt[], color: string, width: number, alpha: number) => {
        ctx.lineWidth = width;
        ctx.lineCap = "round";
        ctx.lineJoin = "round";
        let open = false;
        for (let i = 0; i < pts.length; i++) {
          const q = pts[i];
          // hide the part that dips behind the globe; fade near the limb
          if (q.z < -0.08) {
            if (open) {
              ctx.stroke();
              open = false;
            }
            continue;
          }
          const fade = Math.min(1, (q.z + 0.08) / 0.25);
          if (!open) {
            ctx.beginPath();
            ctx.strokeStyle = withAlpha(color, alpha * fade);
            ctx.moveTo(q.x, q.y);
            open = true;
          } else {
            ctx.lineTo(q.x, q.y);
          }
        }
        if (open) ctx.stroke();
      };

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

      const onRoute = new Set<string>();
      for (const r of p.routes) for (const hp of r.hops) onRoute.add(hp.name);

      // reachable nodes not on any route
      for (const n of p.nodes) {
        if (onRoute.has(n.name) || !visible(n.pos)) continue;
        const xy = projection(n.pos);
        if (!xy) continue;
        ctx.beginPath();
        ctx.arc(xy[0], xy[1], n.count > 3 ? 2.6 : 2, 0, Math.PI * 2);
        ctx.fillStyle = palette.node;
        ctx.fill();
      }

      // routes: lifted arcs through every hop + one travelling bright segment
      if (p.active) {
        p.routes.forEach((r, ri) => {
          if (r.hops.length === 0) return;
          const color = r.tone === "ok" ? palette.ok : palette.accent;
          const width = Math.min(2.4, 1 + Math.log2(1 + r.conns) * 0.35);
          const legs: ArcPt[][] = [];
          let from = p.self;
          for (const hp of r.hops) {
            legs.push(arc(from, hp.pos));
            from = hp.pos;
          }
          for (const leg of legs) strokePts(leg, color, width, 0.55);

          // travelling segment across the whole route
          const all = legs.flat();
          const speed = 1800 + r.hops.length * 500;
          const t = ((now + ri * 530) / speed) % 1;
          const head = Math.floor(t * (all.length - 1));
          const tail = Math.max(0, head - Math.round(all.length * 0.12));
          strokePts(all.slice(tail, head + 1), color, width + 0.7, 1);

          // hop markers: intermediate hops hollow, exit filled + pulse
          r.hops.forEach((hp, hi) => {
            if (!visible(hp.pos)) return;
            const xy = projection(hp.pos);
            if (!xy) return;
            const isExit = hi === r.hops.length - 1;
            if (isExit) {
              const pulse = 0.5 + 0.5 * Math.sin(now / 400 + ri);
              ctx.beginPath();
              ctx.arc(xy[0], xy[1], 6 + pulse * 4, 0, Math.PI * 2);
              ctx.strokeStyle = withAlpha(color, 0.25 + 0.2 * (1 - pulse));
              ctx.lineWidth = 1;
              ctx.stroke();
              ctx.beginPath();
              ctx.arc(xy[0], xy[1], 3.2, 0, Math.PI * 2);
              ctx.fillStyle = color;
              ctx.fill();
            } else {
              ctx.beginPath();
              ctx.arc(xy[0], xy[1], 3.4, 0, Math.PI * 2);
              ctx.fillStyle = palette.sphere;
              ctx.fill();
              ctx.strokeStyle = color;
              ctx.lineWidth = 1.4;
              ctx.stroke();
            }
          });
        });
      }

      // local machine (drawn last so it sits on top of route starts)
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
  return {
    sphere: cssVar("--t-globe-sea"),
    outline: cssVar("--t-globe-rim"),
    land: cssVar("--t-globe-land"),
    landEdge: cssVar("--t-globe-edge"),
    node: cssVar("--t-fg-3"),
    ok: cssVar("--t-ok"),
    accent: cssVar("--t-accent"),
  };
}
