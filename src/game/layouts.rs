//! Maze layouts (BASIC 1230-1920). Each `lay*` paints one of the eight wall
//! patterns into VRAM with the GW-BASIC drawing primitives on [`super::Game`].
//! The arrow layouts (`lay1810`/`lay1920`) seed the [`super::enemies`] state
//! and prime the first frame.

impl super::Game {
    /// 1230: maze of line segments.
    pub(super) fn lay1230(&mut self) {
        for i in 1..=39 {
            self.locate(8, 1 + i); self.pc(196);
            self.locate(16, 80 - i); self.pc(196);
        }
        for i in 0..=8 {
            self.locate(21 - i, 11); self.pc(179);
            self.locate(3 + i, 70); self.pc(179);
            self.locate(21 - i, 26); self.pc(179);
            self.locate(3 + i, 55); self.pc(179);
            self.locate(15, 22 + i); self.pc(196);
            self.locate(6, 51 + i); self.pc(196);
            self.locate(15, 7 + i); self.pc(196);
            self.locate(6, 66 + i); self.pc(196);
            self.locate(18, 7 + i); self.pc(196);
            self.locate(9, 66 + i); self.pc(196);
            self.locate(18, 22 + i); self.pc(196);
            self.locate(9, 51 + i); self.pc(196);
            for i1 in 6..=10 {
                self.locate(i1, 5 + i * 4); self.pc(179);
                self.locate(8 + i1, 44 + i * 4); self.pc(179);
            }
            self.locate(8, 5 + i * 4); self.pc(197);
            self.locate(16, 44 + i * 4); self.pc(197);
        }
        self.locate(3, 70); self.pc(194);
        self.locate(21, 11); self.pc(193);
        self.locate(3, 55); self.pc(194);
        self.locate(15, 26); self.pc(197);
        self.locate(6, 55); self.pc(197);
        self.locate(18, 26); self.pc(197);
        self.locate(9, 55); self.pc(197);
        self.locate(15, 11); self.pc(197);
        self.locate(6, 70); self.pc(197);
        self.locate(18, 11); self.pc(197);
        self.locate(9, 70); self.pc(197);
        self.locate(21, 26); self.pc(193);
    }
    /// 1400: zigzag + rows of pushable stones.
    pub(super) fn lay1400(&mut self) {
        let mut y = 4;
        while y <= 20 {
            for i in 0..=1 {
                let mut q = 1;
                for a in 1..=6 {
                    if q == 1 {
                        q = 0;
                        y += 1;
                    } else {
                        q = 1;
                        y -= 1;
                    }
                    if y < 21 {
                        self.stone(17 + a + 40 * i, y);
                    }
                }
            }
            y += 2;
        }
        let mut x = 2;
        while x <= 78 {
            for i in 0..=1 {
                let mut yy = 7 + 8 * i;
                self.stone(x, yy);
                yy = 8 + 8 * i;
                x += 1;
                self.stone(x, yy);
                yy = 9 + 8 * i;
                x -= 1;
                self.stone(x, yy);
            }
            x += 2;
        }
    }
    /// 1500: grid of rooms with door gaps.
    pub(super) fn lay1500(&mut self) {
        const DATA: [i32; 52] = [
            15, 5, 6, 10, 9, 35, 6, 20, 9, 75, 6, 40, 9, 55, 6, 70, 9, 65, 18, 10, 15, 55, 18, 20,
            15, 65, 18, 30, 15, 75, 18, 40, 9, 45, 12, 20, 9, 15, 12, 30, 9, 15, 18, 50, 9, 15, 6,
            50, 9, 15, 18, 60,
        ];
        for i in 2..=79 {
            for i1 in 1..=2 {
                self.locate(3 + 6 * i1, i); self.pc(196);
            }
        }
        for i in 4..=20 {
            for i1 in 1..=7 {
                self.locate(3, 10 * i1); self.pc(194);
                self.locate(21, 10 * i1); self.pc(193);
                self.locate(i, 10 * i1); self.pc(179);
                for i2 in 1..=2 {
                    self.locate(3 + 6 * i2, 10 * i1); self.pc(197);
                    self.locate(3 + 6 * i2, 80); self.pc(180);
                    self.locate(3 + 6 * i2, 1); self.pc(195);
                }
            }
        }
        let mut p = 0;
        for _ in 1..=13 {
            let c1 = DATA[p]; let c2 = DATA[p + 1]; let c3 = DATA[p + 2]; let c4 = DATA[p + 3];
            p += 4;
            self.locate(c1, c2); self.ps(" ");
            self.locate(c1, c2 - 1); self.pc(180);
            self.locate(c1, c2 + 2); self.pc(195);
            self.locate(c1, c2 + 1); self.ps(" ");
            self.locate(c3 + 2, c4); self.pc(194);
            self.locate(c3 + 1, c4); self.ps(" ");
            self.locate(c3, c4); self.ps(" ");
            self.locate(c3 - 1, c4); self.pc(193);
        }
    }
    /// 1670: nine vertical walls, each with a 3-cell gap (B array).
    pub(super) fn lay1670(&mut self) {
        for i in 1..=9 {
            self.b[i as usize] = 6 + i;
            self.locate(3, 8 * i); self.pc(194);
            for i1 in 4..=20 {
                self.locate(i1, 8 * i); self.pc(179);
            }
            self.locate(21, 8 * i); self.pc(193);
            let bi = self.b[i as usize];
            self.locate(bi - 1, i * 8); self.pc(193);
            for i1 in 0..=2 {
                self.locate(bi + i1, i * 8); self.ps(" ");
            }
            self.locate(bi + 3, i * 8); self.pc(194);
        }
    }
    /// 1750: walls + stone pattern.
    pub(super) fn lay1750(&mut self) {
        self.lay1670();
        let mut i1 = 4;
        while i1 <= 20 {
            for i2 in 0..=9 {
                self.stone(i2 * 8 + 3, i1);
                self.stone(i2 * 8 + 5, i1);
                if i1 < 20 {
                    self.stone(i2 * 8 + 4, i1 + 1);
                }
            }
            i1 += 2;
        }
    }
    /// 1810: init upward arrows.
    pub(super) fn lay1810(&mut self) {
        let mut i = 2;
        while i <= 79 {
            self.d[i as usize][1] = 5 + (self.rnd() * 14.0).trunc() as i32;
            self.d[i as usize][2] = 32;
            i += 2;
        }
        let _ = self.sub1830();
    }
    /// 1920: init horizontal arrows.
    pub(super) fn lay1920(&mut self) {
        for i in 4..=20 {
            for a in 0..=1 {
                self.d[(i + a * 20) as usize][1] =
                    (self.rnd() * 38.0 * 2.0 + 2.0 + a as f64).round() as i32;
                self.d[(i + a * 20) as usize][2] = 32;
            }
        }
        self.d[12][1] = 14;
        self.d[13][1] = 6;
        self.d[32][1] = 65;
        self.d[33][1] = 55;
        let _ = self.sub1970();
    }
}
