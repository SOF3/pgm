use anyhow::Result;
use arrayvec::ArrayVec;
use crossterm::event::{Event, KeyCode, KeyEvent};
use crossterm::style::{Color, Stylize};
use crossterm::{cursor, style, terminal, QueueableCommand};
use crossterm::{
    event,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand,
};
use rand::Rng;
use std::io::Write;
use std::time::{Duration, Instant};
use std::{io, iter, mem, ops};

const PADDING_TOP: usize = 5;
const PADDING_LEFT: usize = 8;
const PADDING_RIGHT: usize = 5;

const SIGNAL_BACKLOG_LENGTH: usize = 4;
const SIGNAL_BACKLOG_UNIT: Tick = Tick(8);
const FLAG_UPDATED_RATE: f64 = 0.8;
const RANDOM_TICK_PERCENTAGE: usize = 20;
const WORLD_WIDTH: usize = 80;
const WORLD_HEIGHT: usize = 40;
const SIDES: [(isize, isize); 4] = [(1, 0), (-1, 0), (0, 1), (0, -1)];
const TICK_FREQ: Duration = Duration::from_millis(1000);

#[derive(Debug, Clone, Copy)]
struct Tick(u32);
#[derive(Debug, Clone, Copy)]
struct Signal(u16);

#[derive(Clone)]
struct Tile {
    ty: TileType,
    signals: [Signal; SIGNAL_BACKLOG_LENGTH],
    signal_sum: Signal,

    next_signal: Signal,
}

impl Default for Tile {
    fn default() -> Self {
        Self {
            ty: TileType::Air,
            signals: [Signal(0); SIGNAL_BACKLOG_LENGTH],
            signal_sum: Signal(0),
            next_signal: Signal(0),
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum TileType {
    Air,
    Bedrock,
    Brick,
}

impl TileType {
    fn rendered(self) -> char {
        match self {
            Self::Air => ' ',
            Self::Bedrock => '=',
            Self::Brick => 'o',
        }
    }

    fn weight(self) -> Signal {
        match self {
            Self::Air => Signal(0),
            Self::Bedrock => Signal(0),
            Self::Brick => Signal(100),
        }
    }

    fn accepts(self) -> bool {
        matches!(self, Self::Bedrock | Self::Brick)
    }
    fn emits(self) -> bool {
        matches!(self, Self::Brick)
    }
    fn absorbs(self) -> bool {
        matches!(self, Self::Bedrock)
    }
}

struct Dim {
    width: usize,
    height: usize,
}

impl Dim {
    fn xy_offset(&self, x: usize, y: usize) -> usize {
        assert!(x < self.width);
        assert!(y < self.height);
        x + y * self.width
    }

    fn offset_xy(&self, offset: usize) -> (usize, usize) {
        assert!(offset < self.width * self.height);
        let x = offset % self.width;
        let y = offset / self.width;
        (x, y)
    }
}

struct World {
    dim: Dim,
    tiles: Vec<Tile>,
    flagged_tiles: Vec<usize>,
    next_flagged_tiles: Vec<usize>,
    cursor: (usize, usize),
}

impl World {
    fn new(width: usize, height: usize) -> Self {
        Self {
            dim: Dim { width, height },
            tiles: iter::repeat(Tile::default()).take(width * height).collect(),
            flagged_tiles: Vec::new(),
            next_flagged_tiles: Vec::new(),
            cursor: (0, 0),
        }
    }

    fn tick(&mut self, now: Tick) {
        self.pre_tick(now);

        let next_flagged_tiles = mem::replace(
            &mut self.next_flagged_tiles,
            Vec::with_capacity(self.flagged_tiles.len()),
        );
        let flagged_tiles = mem::replace(&mut self.flagged_tiles, next_flagged_tiles);
        for flagged in flagged_tiles {
            self.flagged_tick(flagged);
        }
        mem::swap(&mut self.flagged_tiles, &mut self.next_flagged_tiles);
        self.next_flagged_tiles.clear();

        let results = rand::seq::index::sample(
            &mut rand::thread_rng(),
            self.tiles.len(),
            self.tiles.len() * RANDOM_TICK_PERCENTAGE / 100,
        );
        for result in results {
            self.random_tick(result);
        }
    }

    fn pre_tick(&mut self, now: Tick) {
        let current_signal_offset =
            (now.0 / SIGNAL_BACKLOG_UNIT.0) as usize % SIGNAL_BACKLOG_LENGTH;

        for tile in &mut self.tiles {
            tile.signal_sum.0 -= tile.signals[current_signal_offset].0;
            tile.signal_sum.0 += tile.next_signal.0;
            tile.signals[current_signal_offset] = tile.next_signal;
        }
    }

    fn flagged_tick(&mut self, tile_offset: usize) {
        let (x, y) = self.dim.offset_xy(tile_offset);
        let Tile { ty, signal_sum, .. } = self.tiles[tile_offset];
        if ty.emits() {
            let mut conns = ArrayVec::<_, 4>::new();

            for (side, (dx, dy)) in SIDES.into_iter().enumerate() {
                if let (Some(x2), Some(y2)) = (x.checked_add_signed(dx), y.checked_add_signed(dy)) {
                    let neighbor_ty = self[(x2, y2)].ty;
                    if neighbor_ty.accepts() {
                        conns.push(side);
                    }
                }
            }

            self[(x, y)].next_signal.0 -= signal_sum.0;

            let per_side = signal_sum.0 / conns.len() as u16;
            for side in conns {
                let (dx, dy) = SIDES[side];
                let x2 = x.checked_add_signed(dx).unwrap();
                let y2 = y.checked_add_signed(dy).unwrap();
                let neighbor = &mut self[(x2, y2)];
                if !neighbor.ty.absorbs() {
                    neighbor.next_signal.0 += per_side;
                }

                if rand::thread_rng().gen_bool(FLAG_UPDATED_RATE) {
                    self.next_flagged_tiles.push(self.dim.xy_offset(x2, y2));
                }
            }
        }
    }

    fn random_tick(&mut self, tile_offset: usize) {
        let tile = &mut self.tiles[tile_offset];
        tile.next_signal.0 += tile.ty.weight().0;
    }

    fn term_x(&self, x: usize) -> u16 {
        (PADDING_LEFT + x * 2) as u16
    }
    fn term_y(&self, y: usize) -> u16 {
        (PADDING_TOP + self.dim.height - y) as u16
    }

    fn draw(&self) -> Result<()> {
        let mut stdout = io::stdout();
        stdout.queue(terminal::Clear(terminal::ClearType::All))?;

        for y in 0..self.dim.height {
            stdout
                .queue(cursor::MoveTo(1, self.term_y(y)))?
                .queue(style::Print(y))?;
        }

        let x_term_y = (self.dim.height + PADDING_TOP + 2) as u16;
        for x in (0..self.dim.width).step_by(10) {
            stdout
                .queue(cursor::MoveTo(self.term_x(x), x_term_y))?
                .queue(style::Print(x))?;
        }

        let max_signal_sum = self
            .tiles
            .iter()
            .map(|tile| tile.signal_sum)
            .max_by_key(|signal| signal.0)
            .unwrap();

        for x in 0..self.dim.width {
            for y in 0..self.dim.height {
                let tile = &self[(x, y)];

                stdout
                    .queue(cursor::MoveTo(self.term_x(x), self.term_y(y)))?
                    .queue(style::PrintStyledContent(tile.ty.rendered().with(viridis(
                        tile.signal_sum.0 as f64 / max_signal_sum.0 as f64,
                    ))))?;
            }
        }

        let colormap_term_x = (PADDING_LEFT + self.dim.width * 2 + PADDING_RIGHT) as u16;
        for y in 0..self.dim.height {
            let ratio = y as f64 / self.dim.height as f64;
            let signal_value = ratio * max_signal_sum.0 as f64;
            stdout
                .queue(cursor::MoveTo(colormap_term_x, self.term_y(y)))?
                .queue(style::PrintStyledContent(
                    format!("{signal_value:.1}").on(viridis(ratio)),
                ))?;
        }

        stdout.queue(cursor::MoveTo(
            self.term_x(self.cursor.0),
            self.term_y(self.cursor.1),
        ))?;

        stdout.flush()?;

        Ok(())
    }
}

impl ops::Index<(usize, usize)> for World {
    type Output = Tile;

    fn index(&self, (x, y): (usize, usize)) -> &Tile {
        let offset = self.dim.xy_offset(x, y);
        &self.tiles[offset]
    }
}

impl ops::IndexMut<(usize, usize)> for World {
    fn index_mut(&mut self, (x, y): (usize, usize)) -> &mut Tile {
        let offset = self.dim.xy_offset(x, y);
        &mut self.tiles[offset]
    }
}

fn viridis(f: f64) -> Color {
    let mut color = colorgrad::viridis().at(f);
    for comp in [&mut color.r, &mut color.g, &mut color.b] {
        *comp *= 0.5;
        *comp += 0.5;
    }
    Color::Rgb {
        r: (color.r * 255.0) as u8,
        g: (color.g * 255.0) as u8,
        b: (color.b * 255.0) as u8,
    }
}

fn main() -> Result<()> {
    enable_raw_mode()?;
    io::stdout().execute(EnterAlternateScreen)?;

    let mut world = World::new(WORLD_WIDTH, WORLD_HEIGHT);
    for x in 0..world.dim.width {
        world[(x, 0)].ty = TileType::Bedrock;
    }

    let mut next_tick_time = Instant::now();
    let mut current_tick = Tick(0);

    loop {
        if next_tick_time < Instant::now() {
            next_tick_time = Instant::now() + TICK_FREQ;
            world.tick(current_tick);
            current_tick.0 += 1;
        }

        world.draw()?;

        if event::poll(next_tick_time.saturating_duration_since(Instant::now()))? {
            match event::read()? {
                Event::Key(KeyEvent {
                    code: KeyCode::Char('q'),
                    ..
                }) => break,
                Event::Key(KeyEvent {
                    code: KeyCode::Char(ch @ ('h' | 'l' | 'j' | 'k')),
                    ..
                }) => {
                    let (cursor, limit, delta) = match ch {
                        'h' => (&mut world.cursor.0, world.dim.width, -1),
                        'l' => (&mut world.cursor.0, world.dim.width, 1),
                        'j' => (&mut world.cursor.1, world.dim.height, -1),
                        'k' => (&mut world.cursor.1, world.dim.height, 1),
                        _ => unreachable!(),
                    };
                    match (*cursor).checked_add_signed(delta) {
                        None => {}
                        Some(new_value) if new_value >= limit => {}
                        Some(new_value) => *cursor = new_value,
                    }
                }
                Event::Key(KeyEvent {
                    code: KeyCode::Char(ch @ ('0' | '9' | '1')),
                    ..
                }) => {
                    let tile = match ch {
                        '0' => TileType::Air,
                        '9' => TileType::Bedrock,
                        '1' => TileType::Brick,
                        _ => unreachable!(),
                    };
                    let cursor = world.cursor;
                    world[cursor].ty = tile;
                }
                Event::Key(KeyEvent {
                    code: KeyCode::Char('t'),
                    ..
                }) => next_tick_time = Instant::now(),
                _ => {}
            }
        }
    }

    disable_raw_mode()?;
    io::stdout().execute(LeaveAlternateScreen)?;

    Ok(())
}
