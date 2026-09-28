import * as THREE from "three";
import { geoEquirectangular, geoPath, type GeoPermissibleObjects } from "d3-geo";
import { feature } from "topojson-client";
import type { GeometryCollection, Topology } from "topojson-specification";
import landTopo from "world-atlas/land-110m.json";
import { onThemeChange } from "../../theme";
import type { GlobeProps } from "../Globe";
import type { LonLat } from "../../lib/geo";
import { liftedArc, lonLatVector, validPosition } from "./geometry";

export type GlobeStatus = "ready" | "lost" | "unavailable";
export interface GlobeScene {
  update(props: GlobeProps): void;
  reset(): void;
  dispose(): void;
}

const topo = landTopo as unknown as Topology<{ land: GeometryCollection }>;
const land = feature(topo, topo.objects.land) as GeoPermissibleObjects;
const rasters = new Map<string, HTMLCanvasElement>();

function readPalette() {
  const css = getComputedStyle(document.documentElement);
  const get = (name: string, fallback: string) => css.getPropertyValue(name).trim() || fallback;
  return {
    sea: get("--t-globe-sea", "#171c25"),
    land: get("--t-globe-land", "#465366"),
    edge: get("--t-globe-edge", "#637187"),
    rim: get("--t-globe-rim", "#557399"),
    node: get("--t-fg-3", "#aab8c8"),
    ok: get("--t-ok", "#69dca5"),
    accent: get("--t-accent", "#619eff"),
  };
}
type Palette = ReturnType<typeof readPalette>;

/** Natural Earth geometry is bundled by world-atlas; no runtime requests. */
function raster(palette: Palette): HTMLCanvasElement {
  const key = [palette.sea, palette.land, palette.edge].join("|");
  const cached = rasters.get(key);
  if (cached) return cached;
  const canvas = document.createElement("canvas");
  canvas.width = 2048; canvas.height = 1024;
  const ctx = canvas.getContext("2d");
  if (!ctx) throw new Error("Canvas 2D unavailable");
  ctx.fillStyle = palette.sea; ctx.fillRect(0, 0, canvas.width, canvas.height);
  const projection = geoEquirectangular().translate([1024, 512]).scale(2048 / (2 * Math.PI));
  const path = geoPath(projection, ctx);
  ctx.beginPath(); path(land);
  ctx.fillStyle = palette.land; ctx.fill();
  ctx.lineWidth = .9; ctx.strokeStyle = palette.edge; ctx.stroke();
  // Bound CPU memory across theme changes. CanvasTexture disposal never mutates these rasters.
  if (rasters.size >= 2) rasters.delete(rasters.keys().next().value!);
  rasters.set(key, canvas);
  return canvas;
}

interface Disposable { dispose(): void }
interface Flight {
  key: string;
  legs: THREE.CatmullRomCurve3[];
  head: THREE.Mesh;
  material: THREE.MeshBasicMaterial;
}

export function createGlobeScene(canvas: HTMLCanvasElement, status: (status: GlobeStatus) => void): GlobeScene {
  const owned = new Set<Disposable>();
  const dynamic = new Set<Disposable>();
  const cleanups: (() => void)[] = [];
  const keep = <T extends Disposable>(resource: T): T => { owned.add(resource); return resource; };
  const transient = <T extends Disposable>(resource: T): T => { dynamic.add(resource); return resource; };
  let renderer: THREE.WebGLRenderer | null = null;
  let disposed = false, contextLost = false, inView = true, raf = 0, lastTime = 0;
  let width = 1, height = 1, phase = 0, yaw = 0, pitch = 0, released = 0;
  let pointer: number | null = null, x = 0, y = 0;
  let current: GlobeProps | null = null, signature = "", home = "";
  let texture: THREE.CanvasTexture | null = null;
  const flights: Flight[] = [];
  const counts = new Map<string, number>();
  const dispose = () => {
    if (disposed) return;
    disposed = true; cancelAnimationFrame(raf); raf = 0;
    cleanups.reverse().forEach(cleanup => cleanup());
    dynamic.forEach(resource => resource.dispose()); dynamic.clear();
    owned.forEach(resource => resource.dispose()); owned.clear();
    renderer?.dispose(); renderer?.forceContextLoss(); renderer = null;
  };

  try {
    renderer = new THREE.WebGLRenderer({ canvas, alpha: true, antialias: true, powerPreference: "low-power" });
    const view = renderer;
    view.setPixelRatio(Math.min(window.devicePixelRatio || 1, 1.75));
    view.setClearColor(0x000000, 0);
    view.outputColorSpace = THREE.SRGBColorSpace;
    const scene = new THREE.Scene();
    const camera = new THREE.PerspectiveCamera(35, 1, .1, 20);
    camera.position.set(0, 0, 3.85);
    const world = new THREE.Group(); scene.add(world);
    const overlay = new THREE.Group(); world.add(overlay);
    const links = new THREE.Group(); overlay.add(links);
    const points = new THREE.Group(); overlay.add(points);
    const local = new THREE.Group(); overlay.add(local);
    let palette = readPalette();
    const surface = keep(new THREE.MeshPhongMaterial({ color: 0xffffff, specular: 0x253449, shininess: 12 }));
    world.add(new THREE.Mesh(keep(new THREE.SphereGeometry(1, 80, 56)), surface));
    const ambient = new THREE.AmbientLight(0xd4deef, 1.35); scene.add(ambient);
    const sun = new THREE.DirectionalLight(0xe3ecff, 1.4); scene.add(sun);
    const atmosphereMaterial = keep(new THREE.ShaderMaterial({
      uniforms: { color: { value: new THREE.Color(palette.rim) } },
      vertexShader: "varying vec3 n;varying vec3 v;void main(){vec4 p=modelViewMatrix*vec4(position,1.0);n=normalize(normalMatrix*normal);v=normalize(-p.xyz);gl_Position=projectionMatrix*p;}",
      fragmentShader: "uniform vec3 color;varying vec3 n;varying vec3 v;void main(){float f=pow(1.0-max(dot(normalize(n),normalize(v)),0.0),4.0);gl_FragColor=vec4(color,f*.22);}",
      transparent: true, depthWrite: false, blending: THREE.AdditiveBlending,
    }));
    world.add(new THREE.Mesh(keep(new THREE.SphereGeometry(1.016, 64, 48)), atmosphereMaterial));
    const dotGeometry = keep(new THREE.SphereGeometry(.009, 12, 8));
    const ringGeometry = keep(new THREE.RingGeometry(.019, .023, 40));
    const particleGeometry = keep(new THREE.SphereGeometry(.005, 10, 8));
    const lightTheme = () => document.documentElement.dataset.theme === "light";

    const setTexture = () => {
      const next = keep(new THREE.CanvasTexture(raster(palette)));
      next.colorSpace = THREE.SRGBColorSpace;
      next.anisotropy = Math.min(view.capabilities.getMaxAnisotropy(), 8);
      surface.map = next; surface.needsUpdate = true;
      if (texture) { texture.dispose(); owned.delete(texture); }
      texture = next;
      atmosphereMaterial.uniforms.color.value.set(palette.rim);
      ambient.intensity = lightTheme() ? 1.7 : 1.35;
      sun.intensity = lightTheme() ? .75 : 1.4;
    };
    setTexture();

    const marker = (group: THREE.Group, pos: LonLat, color: string, ring: boolean, filled = true, scale = 1) => {
      const direction = lonLatVector(pos);
      const material = transient(new THREE.MeshBasicMaterial({ color, transparent: true, opacity: .95 }));
      if (filled) {
        const dot = new THREE.Mesh(dotGeometry, material);
        dot.position.copy(direction).multiplyScalar(1.012); dot.scale.setScalar(scale); group.add(dot);
      }
      if (ring) {
        const rim = new THREE.Mesh(ringGeometry, material);
        rim.position.copy(direction).multiplyScalar(1.012);
        rim.quaternion.setFromUnitVectors(new THREE.Vector3(0, 0, 1), direction);
        group.add(rim);
      }
    };

    const rebuild = (data: GlobeProps) => {
      links.clear(); points.clear(); local.clear(); flights.length = 0;
      dynamic.forEach(resource => resource.dispose()); dynamic.clear();
      const used = new Set(data.routes.flatMap(route => route.hops.map(hop => hop.name)));
      data.nodes.forEach(node => {
        if (validPosition(node.pos) && !used.has(node.name)) marker(points, node.pos, palette.node, false, true, node.count > 3 ? .85 : .65);
      });
      if (validPosition(data.self)) marker(local, data.self, palette.ok, true);
      for (const route of data.routes) {
        const legs: THREE.CatmullRomCurve3[] = [];
        let from: LonLat | null = validPosition(data.self) ? data.self : null;
        const color = route.tone === "ok" ? palette.ok : palette.accent;
        const material = transient(new THREE.MeshBasicMaterial({ color, transparent: true, opacity: .85 }));
        const glow = transient(new THREE.MeshBasicMaterial({ color, transparent: true, opacity: .08, depthWrite: false }));
        route.hops.forEach((hop, index) => {
          if (!validPosition(hop.pos)) { from = null; return; }
          if (from && lonLatVector(from).distanceToSquared(lonLatVector(hop.pos)) > 1e-10) {
            const curve = new THREE.CatmullRomCurve3(liftedArc(from, hop.pos));
            legs.push(curve);
            links.add(new THREE.Mesh(transient(new THREE.TubeGeometry(curve, 64, .0024, 5, false)), material));
            links.add(new THREE.Mesh(transient(new THREE.TubeGeometry(curve, 64, .006, 5, false)), glow));
          }
          marker(links, hop.pos, color, true, index === route.hops.length - 1);
          from = hop.pos;
        });
        if (legs.length) {
          const particleMaterial = transient(new THREE.MeshBasicMaterial({ color: lightTheme() ? color : "#daebff" }));
          const head = new THREE.Mesh(particleGeometry, particleMaterial); links.add(head);
          flights.push({ key: route.key, legs, head, material });
        }
      }
    };

    const reduced = window.matchMedia("(prefers-reduced-motion: reduce)");
    const visible = () => !disposed && !contextLost && !document.hidden && inView && width > 0 && height > 0;
    const animating = () => !!current?.active && !reduced.matches;
    const frame = (now: number) => {
      raf = 0;
      if (!visible()) return;
      const dt = lastTime ? Math.min((now - lastTime) / 1000, .1) : 0;
      // Cap continuous animation at 30fps, but keep event-driven inactive frames immediate.
      if (animating() && lastTime && now - lastTime < 30) { raf = requestAnimationFrame(frame); return; }
      lastTime = now;
      if (animating() && pointer === null) {
        phase += dt;
        if (now - released > 3500) {
          const decay = Math.exp(-dt * .55); yaw *= decay; pitch *= decay;
        }
      }
      world.rotation.set(pitch, yaw + (animating() ? Math.sin(phase * .12) * .11 : 0), 0);
      flights.forEach((flight, index) => {
        const count = counts.get(flight.key) ?? 0;
        flight.material.opacity = count > 0 ? .92 : .65;
        flight.head.visible = animating() && count > 0;
        const progress = ((phase * (.20 + Math.min(8, Math.log2(1 + count)) * .015) + index * .19) % 1) * flight.legs.length;
        flight.legs[Math.floor(progress)].getPoint(progress % 1, flight.head.position);
      });
      view.render(scene, camera);
      if (animating()) raf = requestAnimationFrame(frame);
    };
    const request = () => {
      if (!visible()) { cancelAnimationFrame(raf); raf = 0; lastTime = 0; return; }
      if (!raf) raf = requestAnimationFrame(frame);
    };
    const resize = () => {
      if (disposed || contextLost) return;
      const rect = canvas.getBoundingClientRect(); width = rect.width; height = rect.height;
      if (width <= 0 || height <= 0) { request(); return; }
      view.setPixelRatio(Math.min(window.devicePixelRatio || 1, 1.75));
      view.setSize(width, height, false); camera.aspect = width / height; camera.updateProjectionMatrix(); request();
    };
    const reset = () => { yaw = 0; pitch = 0; phase = 0; released = performance.now(); request(); };
    const onDown = (event: PointerEvent) => {
      if (event.button !== 0 || pointer !== null || contextLost) return;
      pointer = event.pointerId; x = event.clientX; y = event.clientY;
      canvas.setPointerCapture(event.pointerId); canvas.focus({ preventScroll: true });
    };
    const onMove = (event: PointerEvent) => {
      if (event.pointerId !== pointer) return;
      const factor = 2 / Math.max(120, Math.min(width, height));
      yaw += (event.clientX - x) * factor;
      pitch = THREE.MathUtils.clamp(pitch + (event.clientY - y) * factor, -1, 1);
      x = event.clientX; y = event.clientY; request();
    };
    const onUp = (event: PointerEvent) => {
      if (event.pointerId !== pointer) return;
      pointer = null; released = performance.now();
      if (canvas.hasPointerCapture(event.pointerId)) canvas.releasePointerCapture(event.pointerId);
      request();
    };
    const onKey = (event: KeyboardEvent) => {
      if (!["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown", "Home"].includes(event.key)) return;
      event.preventDefault(); released = performance.now();
      if (event.key === "Home") reset();
      else {
        yaw += event.key === "ArrowLeft" ? -.1 : event.key === "ArrowRight" ? .1 : 0;
        pitch = THREE.MathUtils.clamp(pitch + (event.key === "ArrowUp" ? -.08 : event.key === "ArrowDown" ? .08 : 0), -1, 1);
        request();
      }
    };
    const onLost = (event: Event) => {
      event.preventDefault(); contextLost = true; pointer = null; request(); status("lost");
    };
    const onRestored = () => {
      if (disposed) return;
      contextLost = false; if (texture) texture.needsUpdate = true;
      resize(); request(); status("ready");
    };
    canvas.addEventListener("pointerdown", onDown); canvas.addEventListener("pointermove", onMove);
    canvas.addEventListener("pointerup", onUp); canvas.addEventListener("pointercancel", onUp); canvas.addEventListener("lostpointercapture", onUp);
    canvas.addEventListener("keydown", onKey); canvas.addEventListener("webglcontextlost", onLost); canvas.addEventListener("webglcontextrestored", onRestored);
    cleanups.push(() => {
      canvas.removeEventListener("pointerdown", onDown); canvas.removeEventListener("pointermove", onMove);
      canvas.removeEventListener("pointerup", onUp); canvas.removeEventListener("pointercancel", onUp); canvas.removeEventListener("lostpointercapture", onUp);
      canvas.removeEventListener("keydown", onKey); canvas.removeEventListener("webglcontextlost", onLost); canvas.removeEventListener("webglcontextrestored", onRestored);
      if (pointer !== null && canvas.hasPointerCapture(pointer)) canvas.releasePointerCapture(pointer);
    });
    const resized = new ResizeObserver(resize); resized.observe(canvas); cleanups.push(() => resized.disconnect());
    if (typeof IntersectionObserver !== "undefined") {
      const intersection = new IntersectionObserver(([entry]) => { inView = entry.isIntersecting; request(); });
      intersection.observe(canvas); cleanups.push(() => intersection.disconnect());
    }
    document.addEventListener("visibilitychange", request); reduced.addEventListener("change", request);
    cleanups.push(() => { document.removeEventListener("visibilitychange", request); reduced.removeEventListener("change", request); });
    cleanups.push(onThemeChange(() => {
      if (disposed) return;
      palette = readPalette(); setTexture();
      if (current) rebuild(current);
      request();
    }));
    resize();
    return {
      dispose,
      reset,
      update(data) {
        if (disposed) return;
        current = data;
        const nextHome = data.self.join(",");
        if (nextHome !== home && validPosition(data.self)) {
          home = nextHome;
          camera.position.copy(lonLatVector([data.self[0] + 18, 24], 3.85)); camera.lookAt(0, 0, 0);
          sun.position.copy(camera.position).add(new THREE.Vector3(-1.8, 2.4, 1));
        }
        counts.clear(); data.routes.forEach(route => counts.set(route.key, Math.max(0, route.conns)));
        // Live counts change every second; geometry changes only with actual topology/locations.
        const nextSignature = JSON.stringify([data.self, data.nodes, data.routes.map(({ key, hops, tone }) => ({ key, hops, tone }))]);
        if (signature !== nextSignature) { signature = nextSignature; rebuild(data); }
        links.visible = data.active;
        request();
      },
    };
  } catch (error) {
    dispose();
    throw error;
  }
}
