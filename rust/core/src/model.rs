#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Player {
    One,
    Two,
}

impl Player {
    pub fn other(self) -> Player {
        match self {
            Player::One => Player::Two,
            Player::Two => Player::One,
        }
    }
    pub fn symbol(self) -> char {
        match self {
            Player::One => 'X',
            Player::Two => 'O',
        }
    }
    pub fn index(self) -> usize {
        match self {
            Player::One => 0,
            Player::Two => 1,
        }
    }
}

impl core::fmt::Display for Player {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                Player::One => "ONE",
                Player::Two => "TWO",
            }
        )
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Outcome {
    Win,
    Draw,
    Loss,
}

impl Outcome {
    pub fn from_score_sign(score: i32) -> Outcome {
        if score > 0 {
            Outcome::Win
        } else if score < 0 {
            Outcome::Loss
        } else {
            Outcome::Draw
        }
    }
    pub fn opposite(self) -> Outcome {
        match self {
            Outcome::Win => Outcome::Loss,
            Outcome::Draw => Outcome::Draw,
            Outcome::Loss => Outcome::Win,
        }
    }
}

impl core::fmt::Display for Outcome {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                Outcome::Win => "WIN",
                Outcome::Draw => "DRAW",
                Outcome::Loss => "LOSS",
            }
        )
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Placement {
    Gravity,
    Free,
}

impl core::fmt::Display for Placement {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                Placement::Gravity => "GRAVITY",
                Placement::Free => "FREE",
            }
        )
    }
}

pub mod dirs {
    pub const VERTICAL: u8 = 1;
    pub const HORIZONTAL: u8 = 2;
    pub const DIAGONAL_UP: u8 = 4;
    pub const DIAGONAL_DOWN: u8 = 8;
    pub const ALL: u8 = VERTICAL | HORIZONTAL | DIAGONAL_UP | DIAGONAL_DOWN;
    pub const ORTHOGONAL: u8 = VERTICAL | HORIZONTAL;
}

pub(crate) const DIR_DEFS: [(u8, i32, i32); 4] = [
    (dirs::VERTICAL, 0, 1),
    (dirs::HORIZONTAL, 1, 0),
    (dirs::DIAGONAL_UP, 1, 1),
    (dirs::DIAGONAL_DOWN, 1, -1),
];

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, PartialOrd, Ord)]
pub struct Move {
    pub column: i32,
    pub row: i32,
}

impl Move {
    pub fn column(column: i32) -> Move {
        Move { column, row: -1 }
    }
    pub fn cell(column: i32, row: i32) -> Move {
        Move { column, row }
    }
    pub fn is_resolved(self) -> bool {
        self.row >= 0
    }

    pub fn notation(self) -> String {
        if self.is_resolved() {
            format!("{}:{}", self.column + 1, self.row + 1)
        } else {
            format!("{}", self.column + 1)
        }
    }
}

impl core::fmt::Display for Move {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}", self.notation())
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Rules {
    pub width: i32,
    pub height: i32,
    pub connect: i32,
    pub placement: Placement,
    pub directions: u8,
    pub misere: bool,
    board_mask: Vec<u64>,
    shifts: Vec<u32>,
    dir_steps: Vec<(i32, i32)>,
}

impl Rules {
    pub fn new(
        width: i32,
        height: i32,
        connect: i32,
        placement: Placement,
        directions: u8,
        misere: bool,
    ) -> Result<Rules, String> {
        if directions == 0 || directions & !dirs::ALL != 0 {
            return Err("directions must be a non-empty subset of V,H,DU,DD".into());
        }
        if width < 0 || height < 0 || connect < 0 {
            return Err(format!(
                "width, height and connect (k) must be >= 0, got {width}x{height}, k={connect}"
            ));
        }
        let bits = width as i64 * (height as i64 + 1);
        if bits > 16_384 {
            return Err(format!(
                "board too large for the dense bitboard representation: {bits} bitboard bits (limit 16384); symbolic theory can still classify some larger empty boards"
            ));
        }
        let stride = height.saturating_add(1);
        let word_count = ((bits + 63) / 64) as i32;
        let mut board_mask = vec![0u64; word_count as usize];
        for c in 0..width {
            for r in 0..height {
                let idx = (c * stride + r) as usize;
                board_mask[idx >> 6] |= 1u64 << (idx & 63);
            }
        }
        let mut shifts = Vec::new();
        let mut dir_steps = Vec::new();
        for (bit, dc, dr) in DIR_DEFS {
            if directions & bit != 0 {
                shifts.push(dc.saturating_mul(stride).saturating_add(dr) as u32);
                dir_steps.push((dc, dr));
            }
        }
        Ok(Rules {
            width,
            height,
            connect,
            placement,
            directions,
            misere,
            board_mask,
            shifts,
            dir_steps,
        })
    }

    pub fn standard_connect4() -> Rules {
        Rules::new(7, 6, 4, Placement::Gravity, dirs::ALL, false).unwrap()
    }
    pub fn gravity(
        width: i32,
        height: i32,
        connect: i32,
        diagonals: bool,
    ) -> Result<Rules, String> {
        Rules::new(
            width,
            height,
            connect,
            Placement::Gravity,
            if diagonals {
                dirs::ALL
            } else {
                dirs::ORTHOGONAL
            },
            false,
        )
    }
    pub fn free(width: i32, height: i32, connect: i32, diagonals: bool) -> Result<Rules, String> {
        Rules::new(
            width,
            height,
            connect,
            Placement::Free,
            if diagonals {
                dirs::ALL
            } else {
                dirs::ORTHOGONAL
            },
            false,
        )
    }
    pub fn with_misere(&self, misere: bool) -> Rules {
        let mut r = self.clone();
        r.misere = misere;
        r
    }

    #[inline]
    pub fn stride(&self) -> i32 {
        self.height.saturating_add(1)
    }
    #[inline]
    pub fn cells(&self) -> i32 {
        (self.width as i64 * self.height as i64).clamp(0, i32::MAX as i64) as i32
    }
    #[inline]
    pub fn bit_count(&self) -> i32 {
        (self.width as i64 * self.stride() as i64).clamp(0, i32::MAX as i64) as i32
    }
    #[inline]
    pub fn word_count(&self) -> usize {
        ((self.bit_count() + 63) >> 6) as usize
    }
    #[inline]
    pub fn index(&self, column: i32, row: i32) -> i32 {
        column * self.stride() + row
    }
    #[inline]
    pub fn column_of(&self, index: i32) -> i32 {
        index / self.stride()
    }
    #[inline]
    pub fn row_of(&self, index: i32) -> i32 {
        index % self.stride()
    }
    pub fn board_mask(&self) -> &[u64] {
        &self.board_mask
    }
    pub(crate) fn shifts(&self) -> &[u32] {
        &self.shifts
    }
    pub(crate) fn dir_steps(&self) -> &[(i32, i32)] {
        &self.dir_steps
    }
    pub fn has_diagonals(&self) -> bool {
        self.directions & (dirs::DIAGONAL_UP | dirs::DIAGONAL_DOWN) != 0
    }
    pub fn max_line(&self) -> i32 {
        let mut max_line = 0;
        for &(dc, dr) in &self.dir_steps {
            let len = match (dc, dr) {
                (1, 0) => self.width,
                (0, 1) => self.height,
                _ => self.width.min(self.height),
            };
            max_line = max_line.max(len);
        }
        max_line
    }

    pub fn mirror_symmetric(&self) -> bool {
        (self.directions & dirs::DIAGONAL_UP != 0) == (self.directions & dirs::DIAGONAL_DOWN != 0)
    }
}

impl core::fmt::Display for Rules {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "Connect-{} {}x{} {}{}{}",
            self.connect,
            self.width,
            self.height,
            self.placement,
            if self.has_diagonals() {
                " diagonals"
            } else {
                " no-diagonals"
            },
            if self.misere { " misere" } else { "" }
        )
    }
}
