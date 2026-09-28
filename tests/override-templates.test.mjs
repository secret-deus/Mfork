import test from 'node:test';
import assert from 'node:assert/strict';
import { overrideTemplates, templateYaml } from '../src/lib/override-templates.ts';

test('six unique common template types', () => {
  assert.equal(new Set(overrideTemplates.map(t => t.id)).size, 6);
  assert.deepEqual(overrideTemplates.map(t => t.rule.split(',')[0]), ['DOMAIN','DOMAIN-SUFFIX','DOMAIN-REGEX','DOMAIN-KEYWORD','IP-CIDR','PROCESS-NAME']);
});
for (const template of overrideTemplates) {
  test(`${template.id}: prepend only and quoted YAML scalar round-trips`, () => {
    const yaml = templateYaml(template.rule);
    assert.ok(yaml.startsWith('prepend-rules:\n  - '));
    assert.equal(JSON.parse(yaml.split('\n')[1].slice(4)), template.rule);
    assert.doesNotMatch(yaml, /^rules:/m);
    assert.ok(template.rule.includes(',DIRECT'));
  });
}
test('prefix matches the api. label at the beginning only', () => {
  const pattern = new RegExp(overrideTemplates.find(t => t.id === 'prefix').rule.split(',')[1]);
  for (const domain of ['api.example.com','api.example.org']) assert.ok(pattern.test(domain));
  for (const domain of ['myapi.example.com','www.api.example.com','api-example.com','example.com']) assert.equal(pattern.test(domain), false);
});
test('YAML scalar escapes quotes, slashes and newlines', () => {
  const rule = 'DOMAIN-REGEX,^api\\.,Policy "A"\n';
  const yaml = templateYaml(rule);
  assert.equal(yaml.split('\n').length, 3);
  assert.equal(JSON.parse(yaml.split('\n')[1].slice(4)), rule);
});
test('IP template does not force DNS resolution', () => {
  assert.ok(overrideTemplates.find(t => t.id === 'cidr').rule.endsWith(',no-resolve'));
});
