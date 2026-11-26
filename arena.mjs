import { spawn } from 'node:child_process';
import { appendFileSync, closeSync, mkdirSync, openSync, readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { tmpdir } from 'node:os';
import { basename, join, resolve } from 'node:path';
import { parseArgs } from 'node:util';
import vm from 'node:vm';

const { values: options } = parseArgs({
  options: {
    opponent: { type: 'string', default: 'vct' },
    ms: { type: 'string', default: '1000' },
    set: { type: 'string', default: 'test' },
    pairs: { type: 'string', default: '2' },
    out: { type: 'string' },
    desktop: { type: 'string', default: join(process.env.USERPROFILE, 'Desktop') },
    engine: { type: 'string', default: join(import.meta.dirname, 'gomoku.exe') },
    'wall-ms': { type: 'string' },
    native: { type: 'boolean' },
  },
});
const ms = Number(options.ms);
const pairs = Number(options.pairs);
if (!['vct', 'hybrid', 'katagomo'].includes(options.opponent)
    || !['dev', 'test'].includes(options.set)
    || !Number.isInteger(ms) || ms < 50 || ms > 60000
    || !Number.isInteger(pairs) || pairs < 1 || pairs > 4) {
  throw new Error('Expected --opponent vct|hybrid|katagomo --set dev|test --ms 50..60000 --pairs 1..4');
}

const columns = 'ABCDEFGHJKLMNOP';
const directions = [[1, 0], [0, 1], [1, 1], [1, -1]];
const gomoku = join(options.desktop, '博弈', '棋类', '五子棋');
const sdk = join(gomoku, '算法核心与Katagomo超算SDK');
const openings = {
  dev: ['H8 G7 H6', 'H8 H7 J7 G7 J9', 'H8 G8 J9 H7 G9', 'H8 J9 G7 H9 J7'],
  test: ['H8 H7 G9 J7 F8', 'H8 J8 G8 H9 G6', 'H8 G9 J7 H9 G7', 'H8 J7 F9 H7 G8'],
};
const sleepTolerance = Math.max(90, Math.ceil(ms * 0.15));
const moveLimit = options['wall-ms'] === undefined
  ? (options.native ? Math.max(10_000, ms + 5000) : ms + sleepTolerance)
  : Number(options['wall-ms']);
if (!Number.isInteger(moveLimit) || moveLimit < ms || moveLimit > 120_000) {
  throw new Error('Expected --wall-ms between --ms and 120000');
}
const output = resolve(options.out ?? join(
  import.meta.dirname, 'results', `${options.opponent}-${options.set}-${ms}-${Date.now()}.jsonl`,
));

function fromGtp(text) {
  const m = /^([A-HJ-P])([1-9]|1[0-5])$/i.exec(text);
  if (!m) throw new Error(`Invalid coordinate: ${text}`);
  return { r: 15 - Number(m[2]), c: columns.indexOf(m[1].toUpperCase()) };
}

function toGtp({ r, c } = {}) {
  if (!Number.isInteger(r) || !Number.isInteger(c) || r < 0 || r >= 15 || c < 0 || c >= 15) {
    throw new Error(`Invalid coordinate: ${r},${c}`);
  }
  return columns[c] + (15 - r);
}

function won(board, r, c, p) {
  return directions.some(([dr, dc]) => {
    let count = 1;
    for (const sign of [-1, 1]) {
      let y = r + dr * sign;
      let x = c + dc * sign;
      while (y >= 0 && y < 15 && x >= 0 && x < 15 && board[y * 15 + x] === p) {
        count++;
        y += dr * sign;
        x += dc * sign;
      }
    }
    return count >= 5;
  });
}

function gtp(exe, args, env = process.env, cwd = import.meta.dirname, diagnostics = false) {
  const process = spawn(exe, args, { env, cwd, windowsHide: true });
  let buffer = '';
  let stderr = '';
  let id = 0;
  let pending;
  let failure;
  process.stdout.setEncoding('utf8');
  process.stderr.setEncoding('utf8');
  process.stderr.on('data', chunk => { stderr = (stderr + chunk).slice(-12_000); });
  process.stdin.on('error', error => pending?.reject(error));
  process.on('error', error => { failure = error; pending?.reject(error); });
  process.on('exit', (code, signal) => {
    failure ??= new Error(`${basename(exe)} exited (${code ?? signal}): ${stderr.trim()}`);
    pending?.reject(failure);
  });
  process.stdout.on('data', chunk => {
    buffer += chunk.replace(/\r/g, '');
    let end;
    while ((end = buffer.indexOf('\n\n')) !== -1) {
      const response = buffer.slice(0, end).trim();
      buffer = buffer.slice(end + 2);
      const match = /^([=?])(\d+)\s*([\s\S]*)$/.exec(response);
      if (!match || !pending || Number(match[2]) !== pending.id) continue;
      if (match[1] === '=') pending.resolve(match[3].trim());
      else pending.reject(new Error(match[3]));
    }
  });
  function command(text, timeout = 15_000) {
    if (failure) return Promise.reject(failure);
    if (pending) return Promise.reject(new Error('GTP command already pending'));
    return new Promise((res, rej) => {
      const timer = setTimeout(() => {
        failure = new Error(`GTP timed out: ${text}`);
        pending.reject(failure);
        process.kill();
      }, timeout);
      const finish = fn => value => {
        clearTimeout(timer);
        pending = undefined;
        fn(value);
      };
      pending = { id: ++id, resolve: finish(res), reject: finish(rej) };
      process.stdin.write(`${id} ${text}\n`);
    });
  }
  return {
    command,
    reset: () => command('clear_board'),
    play: ({ p, coord }) => command(`play ${p === 1 ? 'B' : 'W'} ${coord}`),
    async think(p) {
      const move = await command(`genmove ${p === 1 ? 'B' : 'W'}`, moveLimit + 3000);
      return { move };
    },
    detail: diagnostics ? () => command('last_search') : undefined,
    async close() {
      try { await command('quit', 1500); }
      catch { process.kill(); }
      process.stdin.end();
    },
  };
}

function withClock(source) {
  if (options.native) return source;
  const clamped = source
    .replace(/deadline: Date\.now\(\) \+ (\d+)/g, (_, n) =>
      `deadline: Math.min(Date.now() + Math.min(${n}, Math.max(1, Math.floor(__ms * ${n} / 1600))), __deadline)`)
    .replace('this.deadline = Date.now() + allocatedTime;',
      'this.deadline = Math.min(Date.now() + allocatedTime, __deadline);')
    .replace('this.deadline += 1800;', 'this.deadline = Math.min(this.deadline + 1800, __deadline);');
  if (clamped === source || !clamped.includes('allocatedTime, __deadline)')) {
    throw new Error('Legacy search clock could not be adapted');
  }
  return clamped;
}

function javascriptOpponent() {
  const scope = { module: { exports: {} }, __deadline: Infinity, __ms: ms, self: {} };
  const context = vm.createContext(scope);
  let history = [];
  let answer;
  scope.self.postMessage = data => { answer = data.move; };
  const hybrid = options.opponent === 'hybrid';
  let source;
  if (hybrid) {
    const html = readFileSync(join(gomoku, '五子棋_终极AI_双脑混合版.html'), 'utf8');
    const match = /const workerScript = ("(?:[^"\\]|\\[\s\S])*");/.exec(html);
    if (!match) throw new Error('Hybrid Worker source could not be extracted');
    source = JSON.parse(match[1]);
  } else {
    const file = join(sdk, 'vct_engine.js');
    scope.require = createRequire(file);
    source = readFileSync(file, 'utf8');
  }
  vm.runInContext(withClock(source), context);
  const engine = hybrid ? undefined : new scope.module.exports.VctEngine();
  const dispatch = data => scope.self.onmessage({ data });
  const set = ({ r, c, p }) => {
    history.push({ r, c });
    if (hybrid) dispatch({ action: 'set', r, c, p });
    else engine.setAt(r, c, p);
  };
  return {
    reset() {
      history = [];
      if (hybrid) dispatch({ action: 'reset' });
      else engine.reset();
    },
    play: set,
    think(p) {
      scope.__deadline = Date.now() + ms;
      if (hybrid) {
        answer = undefined;
        dispatch({ action: 'think', p, timeMs: ms, maxDepth: 26, historyMoves: history });
      } else {
        answer = engine.think(p, ms, 26);
      }
      return {
        move: toGtp(answer),
        detail: { tactic: answer.tactic, depth: answer.depth, nodes: answer.nodes },
      };
    },
    commit: set,
    close() {},
  };
}

async function makeOpponent() {
  if (options.opponent !== 'katagomo') return javascriptOpponent();
  const packageDir = join(options.desktop, '游戏', 'Katagomo_Lizzieyzy_Package_20210520');
  const engineDir = join(packageDir, 'katago', 'engine');
  const overrides = [
    `maxTime=${Math.max(0.01, (ms - 25) / 1000)}`,
    'ponderingEnabled=false',
    'logAllGTPCommunication=false',
    'logSearchInfo=false',
    'logDir=gomoku-arena-katagomo',
  ].join(',');
  const engine = gtp(join(engineDir, 'freestyle.exe'), [
    'gtp',
    '-config', join(packageDir, 'katago', 'default.cfg'),
    '-model', join(packageDir, 'weights', 'freestyle.bin.gz'),
    '-override-config', overrides,
  ], { ...process.env, Path: `${engineDir};${process.env.Path}` }, tmpdir());
  try {
    await engine.command('name', 120_000);
    await engine.command('boardsize 15');
    await engine.command('clear_board');
    await engine.command('play B H8');
    await engine.command('genmove W', 120_000);
    await engine.command('clear_board');
    return engine;
  } catch (error) {
    await engine.close();
    throw error;
  }
}

async function game(candidate, opponent, opening, candidateColor) {
  const history = [];
  const board = new Uint8Array(225);
  await candidate.reset();
  await opponent.reset();
  const finish = (winner, reason, error) => ({
    opponent: options.opponent,
    openingSet: options.set,
    opening,
    candidateColor,
    targetMs: ms,
    moveLimitMs: moveLimit,
    nativeClock: Boolean(options.native),
    result: winner === 0 ? 'draw' : winner === candidateColor ? 'win' : 'loss',
    reason,
    error,
    moves: history,
  });
  for (const [i, coord] of opening.split(' ').entries()) {
    const p = 1 + i % 2;
    const move = { ...fromGtp(coord), coord, p };
    board[move.r * 15 + move.c] = p;
    await candidate.play(move);
    await opponent.play(move);
    history.push(move);
  }
  while (history.length < 225) {
    const p = 1 + history.length % 2;
    const side = p === candidateColor ? candidate : opponent;
    const other = p === candidateColor ? opponent : candidate;
    let response;
    const start = performance.now();
    try { response = await side.think(p); }
    catch (error) { return finish(3 - p, 'engine-error', error.message); }
    const elapsed = Math.round(performance.now() - start);
    if (elapsed > moveLimit) return finish(3 - p, 'timeout', `${p}: ${elapsed}ms`);
    if (/^resign$/i.test(response.move)) return finish(3 - p, 'resigned');
    let point;
    try { point = fromGtp(response.move); }
    catch (error) { return finish(3 - p, 'illegal-move', error.message); }
    const { r, c } = point;
    if (board[r * 15 + c] !== 0) return finish(3 - p, 'illegal-move', `Occupied: ${response.move}`);
    const move = {
      ...point, coord: response.move, p, ms: elapsed,
      detail: response.detail ?? await side.detail?.(),
    };
    history.push(move);
    board[r * 15 + c] = p;
    if (won(board, r, c, p)) return finish(p, 'five');
    side.commit?.(move);
    await other.play(move);
  }
  return finish(0, 'full-board');
}

const candidate = gtp(resolve(options.engine), [], process.env, import.meta.dirname, true);
let opponent;
let recording;
try {
  mkdirSync(resolve(output, '..'), { recursive: true });
  recording = openSync(output, 'wx');
  await candidate.command('boardsize 15');
  await candidate.command(`time_ms ${Math.max(1, ms - 8)}`);
  const commands = (await candidate.command('list_commands')).split(/\s+/);
  if (!commands.includes('last_search')) candidate.detail = undefined;
  opponent = await makeOpponent();
  const scores = { win: 0, draw: 0, loss: 0 };
  for (const opening of openings[options.set].slice(0, pairs)) {
    for (const color of [1, 2]) {
      const result = await game(candidate, opponent, opening, color);
      appendFileSync(recording, JSON.stringify(result) + '\n');
      scores[result.result]++;
      console.log(`${options.opponent} ${opening} ${color === 1 ? 'B' : 'W'}: ${result.result}, ${result.reason}, ${result.moves.length} plies${result.error ? ' (' + result.error + ')' : ''}`);
    }
  }
  console.log(JSON.stringify({ scores, output }));
} finally {
  if (recording !== undefined) closeSync(recording);
  await opponent?.close();
  await candidate.close();
}
