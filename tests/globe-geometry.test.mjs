import test from 'node:test';
import assert from 'node:assert/strict';
import { liftedArc, lonLatVector, validPosition } from '../src/components/globe/geometry.ts';

const close = (actual, expected, message = '') => assert.ok(Math.abs(actual - expected) < 1e-8, `${message}: ${actual} != ${expected}`);

test('longitude/latitude agree with Three SphereGeometry equirectangular UV orientation', () => {
  const prime = lonLatVector([0, 0]);
  close(prime.x, 1); close(prime.y, 0); close(prime.z, 0);
  const east = lonLatVector([90, 0]); close(east.z, -1);
  const west = lonLatVector([-90, 0]); close(west.z, 1);
  close(lonLatVector([180, 0]).x, -1);
  close(lonLatVector([50, 90]).y, 1);
  close(lonLatVector([50, -90]).y, -1);
  close(lonLatVector([139.69, 35.68], 1.012).length(), 1.012);
});

for (const [label, from, to] of [
  ['nearby', [121.47, 31.23], [139.69, 35.68]],
  ['transpacific', [121.47, 31.23], [-118.24, 34.05]],
  ['antimeridian', [179, 20], [-179, 20]],
  ['same location', [114.17, 22.32], [114.17, 22.32]],
  ['antipodal', [0, 0], [180, 0]],
  ['polar', [0, 90], [0, -90]],
]) {
  test(`lifted arc remains finite, above the sphere and endpoint-correct: ${label}`, () => {
    const arc = liftedArc(from, to);
    assert.equal(arc.length, 65);
    assert.ok(arc[0].distanceTo(lonLatVector(from, 1.012)) < 1e-8);
    assert.ok(arc.at(-1).distanceTo(lonLatVector(to, 1.012)) < 1e-8);
    arc.forEach(point => {
      assert.ok(point.toArray().every(Number.isFinite));
      assert.ok(point.length() >= 1.012 - 1e-9);
      assert.ok(point.length() < 1.35);
    });
    for (let i = 1; i < arc.length; i++) assert.ok(arc[i].distanceTo(arc[i - 1]) < .1);
  });
}

test('route hops join exactly without jumping through the globe', () => {
  const self = [121.47, 31.23], relay = [114.17, 22.32], exit = [139.69, 35.68];
  const first = liftedArc(self, relay), second = liftedArc(relay, exit);
  assert.ok(first.at(-1).distanceTo(second[0]) < 1e-9);
});

test('invalid latitude and nonfinite input are filtered, longitudes wrap naturally', () => {
  assert.equal(validPosition([0, 91]), false);
  assert.equal(validPosition([NaN, 20]), false);
  assert.equal(validPosition([20, Infinity]), false);
  assert.equal(validPosition([540, 30]), true);
  assert.ok(lonLatVector([540, 30]).distanceTo(lonLatVector([180, 30])) < 1e-9);
});
