//! Enemy updates (BASIC 1830-2230): the moving hazards that advance one frame
//! per snake step. Each returns `Err(Death)` when it runs into the snake's head
//! (CP437 `219`/`█`), matching the original's `RETURN 510`.

use super::Death;

impl super::Game {
    /// 1830: arrows (chr 24) climbing up, wrapping 4 -> 21.
    pub(super) fn sub1830(&mut self) -> Result<(), Death> {
        let mut i = 2;
        while i <= 78 {
            let mut i2 = (self.d[i as usize][1] - 1) * 160 + (i - 1) * 2;
            if self.d[i as usize][1] == 4 {
                let v = self.d[i as usize][2] as u8;
                self.poke(i2, v);
                self.poke(i2 + 1, 7);
                self.d[i as usize][1] = 21;
                i2 += 2720;
            }
            if self.peek(i2 - 160) == 219 {
                return Err(Death); // 1860
            }
            if self.peek(i2 - 160) > 100 {
                i += 2;
                continue; // 1870
            }
            if self.d[i as usize][1] != 21 {
                let v = self.d[i as usize][2] as u8;
                self.poke(i2, v);
                self.poke(i2 + 1, 7);
            }
            self.d[i as usize][1] -= 1;
            self.d[i as usize][2] = self.peek(i2 - 160) as i32;
            self.poke(i2 - 160, 24);
            self.poke(i2 - 159, 15);
            i += 2;
        }
        Ok(())
    }
    /// 1970: arrows -> (chr 26) and <- (chr 27) sweeping rows 4-20.
    pub(super) fn sub1970(&mut self) -> Result<(), Death> {
        for i in 4..=20 {
            let mut i2 = (i - 1) * 160 + (self.d[i as usize][1] - 1) * 2;
            if self.d[i as usize][1] == 79 {
                let v = self.d[i as usize][2] as u8;
                self.poke(i2, v);
                self.poke(i2 + 1, 7);
                self.d[i as usize][1] = 1;
                i2 -= 156;
            }
            let mut dd = self.peek(i2 + 2);
            if dd == 219 {
                return Err(Death); // 2000
            }
            if dd == 27 {
                // 2010 head-on quirk
                let v = self.d[(i + 20) as usize][2] as u8;
                self.poke(i2 + 2, v);
                self.d[(i + 20) as usize][2] = 26;
            }
            if dd <= 100 {
                // 2020
                if self.d[i as usize][1] != 1 {
                    let v = self.d[i as usize][2] as u8;
                    self.poke(i2, v);
                    self.poke(i2 + 1, 7);
                }
                self.d[i as usize][1] += 1;
                self.d[i as usize][2] = self.peek(i2 + 2) as i32;
                self.poke(i2 + 2, 26);
                self.poke(i2 + 3, 15);
            }
            let l = i + 20;
            i2 = (i - 1) * 160 + (self.d[l as usize][1] - 1) * 2;
            if self.d[l as usize][1] == 2 {
                let v = self.d[l as usize][2] as u8;
                self.poke(i2, v);
                self.poke(i2 + 1, 7);
                self.d[l as usize][1] = 80;
                i2 += 156;
            }
            dd = self.peek(i2 - 2);
            if dd == 219 {
                return Err(Death); // 2070
            }
            if !(dd > 100 || dd == 26) {
                // 2080
                if self.d[l as usize][1] != 80 {
                    let v = self.d[l as usize][2] as u8;
                    self.poke(i2, v);
                    self.poke(i2 + 1, 7);
                }
                self.d[l as usize][1] -= 1;
                self.d[l as usize][2] = self.peek(i2 - 2) as i32;
                self.poke(i2 - 2, 27);
                self.poke(i2 - 1, 15);
            }
        }
        Ok(())
    }
    /// 2130: gaps crawling down the nine walls (wrap 17 -> 4).
    pub(super) fn sub2130(&mut self) -> Result<(), Death> {
        for d1 in 1..=9 {
            let d2 = (self.b[d1 as usize] - 1) * 160 + (d1 * 8 - 1) * 2;
            if self.b[d1 as usize] == 4 {
                // 2140-2180: wrap case
                let a = self.peek(d2 + 2080) as i32
                    + self.peek(d2 + 2240) as i32
                    + self.peek(d2 + 2400) as i32;
                if a != 96 {
                    continue;
                }
                self.poke(d2 + 2560, 179);
                self.poke(d2 + 2080, 179);
                self.poke(d2 + 2240, 179);
                self.poke(d2 + 2400, 179);
                self.poke(d2, 32);
                self.poke(d2 + 160, 32);
                self.poke(d2 + 320, 32);
                self.poke(d2 + 1920, 179);
            }
            let a = self.peek(d2) as i32 + self.peek(d2 + 160) as i32 + self.peek(d2 + 320) as i32; // 2190
            if a != 96 {
                continue;
            }
            if self.b[d1 as usize] != 4 {
                self.poke(d2 - 160, 179); // 2210
            }
            self.poke(d2, 193);
            self.poke(d2 + 160, 32);
            self.poke(d2 + 320, 32);
            self.poke(d2 + 480, 32);
            self.poke(d2 + 640, 194);
            self.b[d1 as usize] += 1; // 2230
            if self.b[d1 as usize] == 17 {
                self.b[d1 as usize] = 4;
            }
        }
        Ok(())
    }
}
