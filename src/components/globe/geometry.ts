import { Vector3 } from "three";
import type { LonLat } from "../../lib/geo";

/** Matches SphereGeometry UVs and an equirectangular map whose left edge is -180°. */
export function lonLatVector([lon, lat]: LonLat, radius = 1): Vector3 {
  const lambda = lon * Math.PI / 180;
  const phi = lat * Math.PI / 180;
  return new Vector3(Math.cos(phi) * Math.cos(lambda), Math.sin(phi), -Math.cos(phi) * Math.sin(lambda)).multiplyScalar(radius);
}

export function validPosition(pos: LonLat): boolean {
  return Number.isFinite(pos[0]) && Number.isFinite(pos[1]) && Math.abs(pos[1]) <= 90;
}

/** Great-circle interpolation, including coincident and antipodal endpoints. */
export function liftedArc(from: LonLat, to: LonLat, segments = 64): Vector3[] {
  const a = lonLatVector(from);
  const b = lonLatVector(to);
  const angle = Math.acos(Math.max(-1, Math.min(1, a.dot(b))));
  const tangent = b.clone().addScaledVector(a, -a.dot(b));
  if (tangent.lengthSq() < 1e-12) {
    const axis = Math.abs(a.y) < .9 ? new Vector3(0, 1, 0) : new Vector3(1, 0, 0);
    tangent.crossVectors(axis, a).normalize();
  } else tangent.normalize();
  const count = Math.max(2, Math.floor(segments));
  const lift = .055 + .28 * angle / Math.PI;
  return Array.from({ length: count + 1 }, (_, i) => {
    const t = i / count;
    const direction = i === count ? b.clone() : a.clone().multiplyScalar(Math.cos(angle * t)).addScaledVector(tangent, Math.sin(angle * t)).normalize();
    return direction.multiplyScalar(1.012 + Math.sin(Math.PI * t) * lift);
  });
}
