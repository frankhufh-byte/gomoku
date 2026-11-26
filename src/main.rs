use std::io::{self, BufRead, Write};
use std::time::{Duration, Instant};

const SIZE: usize = 15;
const CELLS: usize = SIZE * SIZE;
const NONE: usize = CELLS;
const MATE: i32 = 1_000_000;
const STEPS: [(i32, i32); 4] = [(0, 1), (1, 0), (1, 1), (1, -1)];

#[derive(Clone, Copy, Default)]
struct Feature {
    value: i32,
    fives: u8,
    fours: u8,
    threes: u8,
}

fn patterns() -> Vec<[u8; 2]> {
    let mut table = vec![[0; 2]; 65536];
    for (index, pair) in table.iter_mut().enumerate() {
        for (side, pattern) in pair.iter_mut().enumerate() {
            let mut empty = 0u16;
            let mut own = 1 << 4;
            for i in 0..9 {
                if i == 4 {
                    continue;
                }
                let shift = if i < 4 { i } else { i - 1 };
                match index >> (shift * 2) & 3 {
                    0 => empty |= 1 << i,
                    color if color == side + 1 => own |= 1 << i,
                    _ => {}
                }
            }
            let mut wins = 0;
            let mut longest = 0;
            let mut three = false;
            for start in 0..5 {
                let window = 31 << start;
                if (own | empty) & window != window {
                    continue;
                }
                let count = (own & window).count_ones();
                longest = longest.max(count);
                if count == 4 {
                    wins |= empty & window;
                }
            }
            for start in 1..5 {
                let middle = 15 << start;
                let ends = (1 << (start - 1)) | (1 << (start + 4));
                if empty & ends == ends && (own | empty) & middle == middle {
                    let count = (own & middle).count_ones();
                    three |= count == 3;
                    if count == 2 && longest == 2 {
                        longest = 6;
                    }
                }
            }
            *pattern = match (longest, wins.count_ones(), three) {
                (5, _, _) => 8,
                (_, 2.., _) => 7,
                (_, 1, _) => 6,
                (_, _, true) => 5,
                (3, _, _) => 4,
                (6, _, _) => 3,
                (2, _, _) => 2,
                (1, _, _) => 1,
                _ => 0,
            };
        }
    }
    table
}

fn features() -> Vec<Feature> {
    let weights = [0, 0, 5, 18, 30, 124, 240, 1800, 3400];
    (0..65536)
        .map(|pattern| {
            let mut result = Feature::default();
            let mut twos = 0;
            for d in 0..4 {
                let shape = pattern >> (d * 4) & 15;
                result.value += weights.get(shape).copied().unwrap_or(0);
                match shape {
                    8 => result.fives += 1,
                    7 => result.fours += 2,
                    6 => result.fours += 1,
                    5 => result.threes += 1,
                    3 => twos += 1,
                    _ => {}
                }
            }
            result.value += 22 * (twos * (twos - 1) / 2);
            if result.threes >= 2 {
                result.value += 440;
            }
            if result.fours != 0 && result.threes != 0 {
                result.value += 620;
            }
            if result.fours >= 2 {
                result.value += 1200;
            }
            result
        })
        .collect()
}

#[derive(Clone, Copy)]
struct Link {
    cell: usize,
    dir: usize,
    shift: usize,
}

#[derive(Clone, Copy, Default)]
struct Cell {
    code: [u16; 4],
    shapes: [u16; 2],
    stone: u8,
}

#[derive(Clone, Copy, Default)]
struct Points([u64; 4]);

impl Points {
    fn first(self) -> Option<usize> {
        let mut points = self;
        points.pop()
    }

    fn any(self) -> bool {
        self.0 != [0; 4]
    }
    fn change(&mut self, p: usize, active: bool) {
        let bit = 1u64 << (p & 63);
        let word = &mut self.0[p >> 6];
        *word = (*word & !bit) | if active { bit } else { 0 };
    }

    fn pop(&mut self) -> Option<usize> {
        for (i, word) in self.0.iter_mut().enumerate() {
            if *word != 0 {
                let p = word.trailing_zeros() as usize;
                *word &= *word - 1;
                return Some(i * 64 + p);
            }
        }
        None
    }
}

struct Board {
    cells: [Cell; CELLS],
    links: [Vec<Link>; CELLS],
    patterns: Vec<[u8; 2]>,
    features: Vec<Feature>,
    wins: [Points; 2],
    fours: [Points; 2],
    open: [Points; 2],
    threes: [Points; 2],
    totals: [i32; 2],
    hashes: [[u64; 2]; CELLS],
    hash: u64,
    stones: usize,
}

impl Board {
    fn new() -> Self {
        let mut seed = 0x86e1_54aa_4c34_927du64;
        let mut board = Self {
            cells: [Cell::default(); CELLS],
            links: std::array::from_fn(|_| Vec::new()),
            patterns: patterns(),
            features: features(),
            wins: [Points::default(); 2],
            fours: [Points::default(); 2],
            open: [Points::default(); 2],
            threes: [Points::default(); 2],
            totals: [0; 2],
            hashes: std::array::from_fn(|_| {
                std::array::from_fn(|_| {
                    seed ^= seed >> 12;
                    seed ^= seed << 25;
                    seed ^= seed >> 27;
                    seed.wrapping_mul(0x2545_f491_4f6c_dd1d)
                })
            }),
            hash: 0,
            stones: 0,
        };
        for cell in 0..CELLS {
            let (r, c) = (cell as i32 / SIZE as i32, cell as i32 % SIZE as i32);
            for (dir, (dr, dc)) in STEPS.into_iter().enumerate() {
                for (index, delta) in [-4, -3, -2, -1, 1, 2, 3, 4].into_iter().enumerate() {
                    let (row, col) = (r + dr * delta, c + dc * delta);
                    let shift = index * 2;
                    if (0..SIZE as i32).contains(&row) && (0..SIZE as i32).contains(&col) {
                        board.links[row as usize * SIZE + col as usize].push(Link {
                            cell,
                            dir,
                            shift,
                        });
                    } else {
                        board.cells[cell].code[dir] |= 3 << shift;
                    }
                }
            }
        }
        board
    }

    fn feature(&self, p: usize, side: usize) -> Feature {
        self.features[self.cells[p].shapes[side] as usize]
    }

    fn update_features(&mut self, p: usize, old: [u16; 2], new: [u16; 2]) {
        for side in 0..2 {
            let before = self.features[old[side] as usize];
            let after = self.features[new[side] as usize];
            self.totals[side] += after.value.min(450) - before.value.min(450);
            if before.fives != after.fives {
                self.wins[side].change(p, after.fives != 0);
            }
            if before.fours != after.fours {
                self.fours[side].change(p, after.fours != 0);
                self.open[side].change(p, after.fours >= 2);
            }
            if before.threes != after.threes {
                self.threes[side].change(p, after.threes != 0);
            }
        }
    }

    fn set(&mut self, p: usize, stone: u8) {
        let old = self.cells[p].stone;
        let shapes = self.cells[p].shapes;
        self.cells[p].stone = stone;
        if stone == 0 {
            self.stones -= 1;
            self.hash ^= self.hashes[p][(old - 1) as usize];
            self.update_features(p, [0; 2], shapes);
        } else {
            self.stones += 1;
            self.hash ^= self.hashes[p][(stone - 1) as usize];
            self.update_features(p, shapes, [0; 2]);
        }
        for i in 0..self.links[p].len() {
            let Link { cell, dir, shift } = self.links[p][i];
            let target = &mut self.cells[cell];
            let code = target.code[dir] ^ (u16::from(old ^ stone) << shift);
            target.code[dir] = code;
            let pattern = self.patterns[code as usize];
            let previous = target.shapes;
            let offset = dir * 4;
            let next = [
                (previous[0] & !(15 << offset)) | (u16::from(pattern[0]) << offset),
                (previous[1] & !(15 << offset)) | (u16::from(pattern[1]) << offset),
            ];
            if previous != next {
                target.shapes = next;
                if target.stone == 0 {
                    self.update_features(cell, previous, next);
                }
            }
        }
    }
}

#[derive(Clone, Copy)]
struct Entry {
    key: u64,
    score: i32,
    mv: u8,
    depth: u8,
    bound: u8,
}

#[derive(Default)]
struct Move {
    p: usize,
    priority: i32,
}

struct Engine {
    board: Board,
    table: Vec<Entry>,
    history: [[i32; CELLS]; 2],
    nodes: u64,
    proof_nodes: u64,
    deadline: Instant,
    proof_deadline: Instant,
}

impl Engine {
    fn new() -> Self {
        Self {
            board: Board::new(),
            table: vec![
                Entry {
                    key: 0,
                    score: 0,
                    mv: 0,
                    depth: 0,
                    bound: 0
                };
                1 << 19
            ],
            history: [[0; CELLS]; 2],
            nodes: 0,
            proof_nodes: 0,
            deadline: Instant::now(),
            proof_deadline: Instant::now(),
        }
    }

    fn candidates(&self, side: usize, pv: usize, moves: &mut [Move; CELLS]) -> usize {
        let mut count = 0;
        for p in 0..CELLS {
            if self.board.cells[p].stone != 0 {
                continue;
            }
            let mine = self.board.feature(p, side);
            let theirs = self.board.feature(p, 1 - side);
            let center = 14 - (p / SIZE).abs_diff(7) - (p % SIZE).abs_diff(7);
            let tactical = mine.value.max(theirs.value);
            if tactical == 0 && self.board.stones > 1 && p != pv {
                continue;
            }
            let priority = tactical * 16
                + mine.value * 8
                + theirs.value * 5
                + center as i32 * 3
                + self.history[side][p] / 16
                + if p == pv { 200_000 } else { 0 };
            moves[count] = Move { p, priority };
            count += 1;
        }
        if count == 0 {
            if let Some(p) = self.board.cells.iter().position(|c| c.stone == 0) {
                moves[0] = Move { p, priority: 0 };
                return 1;
            }
        }
        count
    }

    // A returned move is a proof. A miss includes exhausted search budgets.
    fn prove(&mut self, side: usize, depth: i32, allow_threes: bool) -> Option<usize> {
        self.proof_nodes += 1;
        if depth <= 0 || Instant::now() >= self.proof_deadline {
            return None;
        }
        let mut wins = self.board.wins[side];
        if let Some(p) = wins.pop() {
            return Some(p);
        }
        let mut opposing_wins = self.board.wins[1 - side];
        let forced = opposing_wins.pop();
        if opposing_wins.pop().is_some() {
            return None;
        }
        let mut fours = self.board.fours[side];
        let mut threes = self.board.threes[side];
        if let Some(p) = forced {
            let feature = self.board.feature(p, side);
            if feature.fours != 0 || (allow_threes && feature.threes != 0) {
                return self.prove_move(side, depth, allow_threes, p).then_some(p);
            }
            return None;
        }
        if let Some(p) = self.board.open[side].first() {
            return Some(p);
        }
        while let Some(p) = fours.pop() {
            if self.prove_move(side, depth, allow_threes, p) {
                return Some(p);
            }
            threes.change(p, false);
            if Instant::now() >= self.proof_deadline {
                return None;
            }
        }
        if allow_threes {
            while let Some(p) = threes.pop() {
                if self.prove_move(side, depth, true, p) {
                    return Some(p);
                }
                if Instant::now() >= self.proof_deadline {
                    break;
                }
            }
        }
        None
    }

    fn prove_move(&mut self, side: usize, depth: i32, allow_threes: bool, p: usize) -> bool {
        self.board.set(p, side as u8 + 1);
        let mut wins = self.board.wins[side];
        let result = if let Some(block) = wins.pop() {
            if wins.pop().is_some() {
                true
            } else {
                self.prove_reply(side, depth, allow_threes, block)
            }
        } else if let Some(followup) = self.board.open[side].first() {
            // Any other reply leaves both endpoints intact unless it creates
            // a winning point of its own, which also has to be searched.
            let mut replies = self.board.fours[1 - side];
            self.board.set(followup, side as u8 + 1);
            let mut endpoints = self.board.wins[side];
            self.board.set(followup, 0);
            replies.change(followup, true);
            while let Some(end) = endpoints.pop() {
                replies.change(end, true);
            }
            let mut proven = true;
            while let Some(reply) = replies.pop() {
                if !self.prove_reply(side, depth, allow_threes, reply) {
                    proven = false;
                    break;
                }
            }
            proven
        } else {
            false
        };
        self.board.set(p, 0);
        result
    }

    fn prove_reply(&mut self, side: usize, depth: i32, allow_threes: bool, p: usize) -> bool {
        self.board.set(p, (2 - side) as u8);
        let win = self.prove(side, depth - 2, allow_threes).is_some();
        self.board.set(p, 0);
        win
    }

    fn search(
        &mut self,
        side: usize,
        depth: i32,
        ply: i32,
        mut alpha: i32,
        beta: i32,
    ) -> Option<(i32, usize)> {
        self.nodes += 1;
        if self.nodes & 63 == 0 && Instant::now() >= self.deadline {
            return None;
        }
        if let Some(p) = self.board.wins[side].first() {
            return Some((MATE - ply - 1, p));
        }
        let mut threats = self.board.wins[1 - side];
        let forced = threats.pop();
        if threats.pop().is_some() {
            return Some((-MATE + ply + 2, forced.unwrap()));
        }
        if self.board.stones == CELLS {
            return Some((0, NONE));
        }
        if let Some(p) = forced {
            if ply >= 70 {
                return Some((self.evaluate(side), p));
            }
            self.board.set(p, side as u8 + 1);
            let result = self.search(1 - side, (depth - 1).max(0), ply + 1, -beta, -alpha);
            self.board.set(p, 0);
            return result.map(|(score, _)| (-score, p));
        }
        if let Some(p) = self.board.open[side].first() {
            return Some((MATE - ply - 3, p));
        }
        if depth <= 0 {
            return Some((self.evaluate(side), NONE));
        }
        let key = self.board.hash ^ if side == 0 { 0 } else { 0xb37d_a78c_7249_2c13 };
        let index = key as usize & (self.table.len() - 1);
        let entry = self.table[index];
        let matching = entry.bound != 0 && entry.key == key;
        if matching && i32::from(entry.depth) >= depth && ply != 0 {
            let mut score = entry.score;
            if score.abs() > MATE - 1000 {
                score -= score.signum() * ply;
            }
            if entry.bound == 1
                || (entry.bound == 2 && score >= beta)
                || (entry.bound == 3 && score <= alpha)
            {
                return Some((score, usize::from(entry.mv)));
            }
        }
        let mut moves = std::array::from_fn(|_| Move::default());
        let count = self.candidates(
            side,
            if matching { entry.mv as usize } else { NONE },
            &mut moves,
        );
        let limit = count.min(if ply == 0 {
            30
        } else if ply <= 2 {
            17
        } else {
            11
        });
        let original_alpha = alpha;
        let mut best = (-MATE, moves[0].p);
        for i in 0..limit {
            let mut top = i;
            for j in (i + 1)..count {
                if moves[j].priority > moves[top].priority {
                    top = j;
                }
            }
            moves.swap(i, top);
            let p = moves[i].p;
            let mine = self.board.feature(p, side);
            let theirs = self.board.feature(p, 1 - side);
            let reduction = i32::from(
                i >= 5
                    && depth >= 3
                    && mine.fours == 0
                    && mine.threes == 0
                    && theirs.fours == 0
                    && theirs.threes < 2,
            );
            self.board.set(p, side as u8 + 1);
            let mut result = if i == 0 {
                self.search(1 - side, depth - 1, ply + 1, -beta, -alpha)
            } else {
                self.search(1 - side, depth - 1 - reduction, ply + 1, -alpha - 1, -alpha)
            };
            if i != 0 && result.is_some_and(|(s, _)| -s > alpha && (reduction != 0 || -s < beta)) {
                result = self.search(1 - side, depth - 1, ply + 1, -beta, -alpha);
            }
            self.board.set(p, 0);
            let score = -result?.0;
            if score > best.0 {
                best = (score, p);
            }
            alpha = alpha.max(score);
            if alpha >= beta {
                let value = &mut self.history[side][p];
                *value += depth * depth * 8 * (16_384 - *value) / 16_384;
                break;
            }
        }
        let (score, mv) = best;
        let bound = if score >= beta {
            2
        } else if score <= original_alpha {
            3
        } else {
            1
        };
        self.table[index] = Entry {
            key,
            score: if score.abs() > MATE - 1000 {
                score + score.signum() * ply
            } else {
                score
            },
            mv: mv as u8,
            depth: depth as u8,
            bound,
        };
        Some(best)
    }

    fn evaluate(&self, side: usize) -> i32 {
        let opponent = 1 - side;
        let mut score = self.board.totals[side] - self.board.totals[opponent];
        if self.board.open[opponent].any() {
            score -= 2300;
        }
        if self.board.fours[side].any() {
            score += 110;
        }
        score
    }

    fn choose(&mut self, side: usize, millis: u64) -> (usize, u8, i32, bool) {
        let start = Instant::now();
        self.deadline = start + Duration::from_millis(millis);
        self.nodes = 0;
        self.proof_nodes = 0;
        self.history = [[0; CELLS]; 2];
        if self.board.stones == 0 {
            return (CELLS / 2, 0, 0, false);
        }
        if let Some(p) = self.board.wins[side].first() {
            return (p, 0, MATE - 1, true);
        }
        if let Some(p) = self.board.wins[1 - side].first() {
            return (p, 0, 0, false);
        }
        if self.board.stones >= 5 {
            self.proof_deadline = start + Duration::from_millis((millis / 20).clamp(1, 120));
            if let Some(p) = self.prove(side, 23, false) {
                return (p, 0, MATE, true);
            }
            self.proof_deadline = start + Duration::from_millis((millis / 5).clamp(1, 850));
            for depth in [5, 9, 13, 19] {
                if let Some(p) = self.prove(side, depth, true) {
                    return (p, 0, MATE, true);
                }
                if Instant::now() >= self.proof_deadline {
                    break;
                }
            }
        }
        let mut moves = std::array::from_fn(|_| Move::default());
        let count = self.candidates(side, NONE, &mut moves);
        let mut best = (
            moves[..count]
                .iter()
                .max_by_key(|m| m.priority)
                .map_or(NONE, |m| m.p),
            0,
            self.evaluate(side),
            false,
        );
        for depth in 1..=28 {
            if Instant::now() >= self.deadline {
                break;
            }
            if let Some((score, p)) = self.search(side, depth, 0, -MATE, MATE) {
                best = (p, depth as u8, score, false);
                if score >= MATE - 100 || (score <= -MATE + 100 && depth >= 6) {
                    break;
                }
            } else {
                break;
            }
        }
        best
    }
}

fn coord(p: usize) -> String {
    if p == NONE {
        return "pass".into();
    }
    format!(
        "{}{}",
        b"ABCDEFGHJKLMNOP"[p % SIZE] as char,
        SIZE - p / SIZE
    )
}

fn parse_coord(s: &str) -> Option<usize> {
    let s = s.to_ascii_uppercase();
    let col = b"ABCDEFGHJKLMNOP"
        .iter()
        .position(|c| Some(c) == s.as_bytes().first())?;
    let row = s.get(1..)?.parse::<usize>().ok()?;
    (1..=SIZE).contains(&row).then(|| (SIZE - row) * SIZE + col)
}

fn color(s: &str) -> Option<usize> {
    match s.to_ascii_lowercase().as_str() {
        "b" | "black" => Some(0),
        "w" | "white" => Some(1),
        _ => None,
    }
}

fn main() -> io::Result<()> {
    let mut engine = Engine::new();
    let mut moves = Vec::new();
    let mut millis = 1000;
    let mut last_search = String::new();
    let mut stdout = io::stdout().lock();
    for line in io::stdin().lock().lines() {
        let line = line?;
        // PowerShell 5.1 prefixes the first line of a pipe with a UTF-8 BOM,
        // which would turn `boardsize` into an unknown command. Strip it.
        let line = line.trim_start_matches('\u{feff}');
        let mut tokens = line.split_whitespace().peekable();
        let id = if tokens.peek().is_some_and(|s| s.parse::<u32>().is_ok()) {
            tokens.next().unwrap()
        } else {
            ""
        };
        let Some(command) = tokens.next() else {
            continue;
        };
        let result: Result<String, &str> = (|| {
            match command {
            "name" => Ok("Gomoku Rust".into()),
            "version" => Ok(env!("CARGO_PKG_VERSION").into()),
            "protocol_version" => Ok("2".into()),
            "list_commands" => Ok("name\nversion\nprotocol_version\nlist_commands\nboardsize\nclear_board\nplay\ngenmove\nundo\ntime_ms\nlast_search\nquit".into()),
            "boardsize" if tokens.next() == Some("15") => Ok(String::new()),
            "clear_board" => {
                while let Some(p) = moves.pop() {
                    engine.board.set(p, 0);
                }
                last_search.clear();
                Ok(String::new())
            }
            "undo" => {
                let p = moves.pop().ok_or("empty board")?;
                engine.board.set(p, 0);
                Ok(String::new())
            }
            "time_ms" => {
                millis = tokens.next().and_then(|s| s.parse().ok())
                    .filter(|n| (1..=600_000).contains(n)).ok_or("invalid time")?;
                Ok(String::new())
            }
            "play" => {
                let side = tokens.next().and_then(color).ok_or("invalid color")?;
                let p = tokens.next().and_then(parse_coord).ok_or("invalid coordinate")?;
                if engine.board.cells[p].stone != 0 {
                    return Err("occupied");
                }
                engine.board.set(p, side as u8 + 1);
                moves.push(p);
                Ok(String::new())
            }
            "genmove" => {
                let side = tokens.next().and_then(color).ok_or("invalid color")?;
                let (p, depth, score, proof) = engine.choose(side, millis);
                if p != NONE {
                    engine.board.set(p, side as u8 + 1);
                    moves.push(p);
                }
                last_search = format!(
                    "move={} depth={depth} score={score} nodes={} proof_nodes={} proven={proof}",
                    coord(p), engine.nodes, engine.proof_nodes
                );
                Ok(coord(p))
            }
            "last_search" => Ok(last_search.clone()),
            "quit" => Ok(String::new()),
            _ => Err("unknown or unsupported command"),
        }
        })();
        match result {
            Ok(value) => writeln!(stdout, "={id} {value}\n")?,
            Err(error) => writeln!(stdout, "?{id} {error}\n")?,
        }
        stdout.flush()?;
        if command == "quit" {
            break;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn makes_five(board: &Board, p: usize, side: usize) -> bool {
        STEPS.into_iter().any(|(dr, dc)| {
            let mut count = 1;
            for sign in [-1, 1] {
                let mut r = (p / SIZE) as i32 + sign * dr;
                let mut c = (p % SIZE) as i32 + sign * dc;
                while (0..SIZE as i32).contains(&r)
                    && (0..SIZE as i32).contains(&c)
                    && board.cells[r as usize * SIZE + c as usize].stone == side as u8 + 1
                {
                    count += 1;
                    r += sign * dr;
                    c += sign * dc;
                }
            }
            count >= 5
        })
    }

    fn validate(board: &Board) {
        for side in 0..2 {
            let mut wins = Points::default();
            let mut open = Points::default();
            let mut total = 0;
            for (p, cell) in board.cells.iter().enumerate() {
                if cell.stone != 0 {
                    continue;
                }
                let feature = board.feature(p, side);
                assert_eq!(feature.fives != 0, makes_five(board, p, side));
                wins.change(p, feature.fives != 0);
                open.change(p, feature.fours >= 2);
                total += feature.value.min(450);
            }
            assert_eq!(board.wins[side].0, wins.0);
            assert_eq!(board.open[side].0, open.0);
            assert_eq!(board.totals[side], total);
        }
    }

    #[test]
    fn incremental_state_matches_direct_scan_after_play_and_undo() {
        let mut board = Board::new();
        for i in 0..160 {
            board.set((i * 67) % CELLS, (i % 2 + 1) as u8);
            validate(&board);
        }
        for i in (0..160).rev() {
            board.set((i * 67) % CELLS, 0);
            validate(&board);
        }
        assert_eq!(board.hash, 0);
        assert_eq!(board.stones, 0);
    }

    #[test]
    fn recognizes_overline_and_blocks_single_win() {
        let mut engine = Engine::new();
        for p in ["E8", "F8", "G8", "J8", "K8"] {
            engine.board.set(parse_coord(p).unwrap(), 1);
        }
        let gap = parse_coord("H8").unwrap();
        assert_eq!(engine.choose(0, 10).0, gap);
        assert_eq!(engine.choose(1, 10).0, gap);
        validate(&engine.board);
    }

    #[test]
    fn proves_crossing_threes_and_respects_counter_win() {
        let mut engine = Engine::new();
        for p in ["G8", "J8", "H7", "H9"] {
            engine.board.set(parse_coord(p).unwrap(), 1);
        }
        let cross = parse_coord("H8").unwrap();
        engine.proof_deadline = Instant::now() + Duration::from_secs(1);
        assert!(engine.prove_move(0, 7, true, cross));
        for p in ["A1", "B1", "C1", "D1"] {
            engine.board.set(parse_coord(p).unwrap(), 2);
        }
        assert_eq!(engine.prove(0, 7, true), None);
        assert_eq!(engine.choose(0, 10).0, parse_coord("E1").unwrap());
        validate(&engine.board);
    }
}
