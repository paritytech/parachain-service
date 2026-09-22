#!/usr/bin/env node
// Quint 0.32.0's CLI simulator/ITF implementation, with stdout transport.
// These are internal APIs: fail closed on a version change and test against CLI ITF.
const fs = require('node:fs');
const path = require('node:path');
const { once } = require('node:events');

// Rust owns campaign progress. Disable Quint's terminal bar, preserving stderr diagnostics.
process.stderr.isTTY = false;

function quintRoot() {
  if (process.env.QUINT_PACKAGE) return process.env.QUINT_PACKAGE;
  const executable = (process.env.PATH || '').split(path.delimiter)
    .map(dir => path.join(dir, 'quint')).find(file => fs.existsSync(file));
  if (!executable) throw new Error('quint is missing from PATH (or set QUINT_PACKAGE)');
  return path.resolve(path.dirname(fs.realpathSync(executable)), '../..');
}
function unwrap(result) {
  if (result.isLeft()) throw new Error(JSON.stringify(result.value, (_, v) =>
    typeof v === 'bigint' ? v.toString() : v));
  return result.value;
}
async function main() {
  const root = quintRoot();
  const version = require(path.join(root, 'package.json')).version;
  if (version !== '0.32.0') throw new Error(`Expected Quint 0.32.0, found ${version}`);
  const api = name => require(path.join(root, 'dist/src', name));
  const cli = api('cliCommands');
  const { toExpr } = api('cliHelpers');
  const { Evaluator } = api('runtime/impl/evaluator');
  const { newTraceRecorder } = api('runtime/trace');
  const { newRng } = api('rng');
  const { toItf } = api('itf');
  const [input, firstSeed, strideText, countText, stepsText] = process.argv.slice(2);
  const stride = BigInt(strideText);
  const count = Number(countText), steps = Number(stepsText);
  if (stride < 1n || !Number.isSafeInteger(count) || count < 0 ||
      !Number.isSafeInteger(steps) || steps < 1) throw new Error('invalid stream limits');
  const args = { input, main: 'fuzz', verbosity: 0 };
  const loaded = unwrap(await cli.load(args));
  const parsed = unwrap(await cli.parse(loaded));
  const typed = unwrap(await cli.typecheck(parsed));
  const [init, step, invariant] = ['replayInit', 'replayStep', 'true'].map(e => unwrap(toExpr(typed, e)));
  for (let index = 0, seed = BigInt(firstSeed); count === 0 || index < count; index++, seed += stride) {
    const rng = newRng(seed);
    const recorder = newTraceRecorder(0, rng, 1);
    const evaluator = new Evaluator(typed.resolver.table, recorder, rng, false);
    let trace;
    const start = performance.now();
    const outcome = evaluator.simulate(init, step, invariant, [], 1, steps, 1,
      (_index, status, vars, states) => {
        if (status !== 'ok') throw new Error(`Quint simulation status: ${status}`);
        trace = unwrap(toItf(vars, states, false));
      });
    if (outcome.status !== 'ok' || !trace || trace.states.length !== steps + 1)
      throw new Error(`seed ${seed}: simulation failed or stopped early (${outcome.status})`);
    // Strip only ITF metadata; preserve all generated model values.
    delete trace['#meta'];
    for (const state of trace.states) delete state['#meta'];
    const line = JSON.stringify({ seed: seed.toString(), steps, version,
      generation_ms: performance.now() - start, trace }) + '\n';
    // OS pipe + await drain bounds memory even when Rust is slower.
    if (!process.stdout.write(line)) await once(process.stdout, 'drain');
  }
}
main().catch(error => { console.error(error); process.exitCode = 1; });
