//! 平滑滚动与弹性边界。gpui 自带的滚动每格滚轮立即跳到新位置，这里自己管理：
//! 滚轮只设定目标位置，内容用临界阻尼弹簧逐帧滑过去。正常滚动到极限会停住；
//! 已经停在顶端或底端后继续滚，内容按阻尼拉出一段，停手后弹回。
//! Smooth scrolling with elastic edges. gpui's own scrolling jumps a whole wheel notch at once,
//! so this view scrolls itself: the wheel only sets a target and a critically damped spring
//! carries the content there frame by frame. Scrolling stops at the limits; scrolling on from a
//! limit pulls the content out against resistance, and it springs back when the wheel stops.
use gpui::{Pixels, px};
use std::{
    cell::Cell,
    rc::Rc,
    time::{Duration, Instant},
};

/// 最远拉出的距离。
/// The furthest the content is pulled past an edge.
const REACH: f32 = 72.;
/// 越拉越难：剩余距离越少，同样的滚动拉出越少。
/// Harder the further it goes: the less room is left, the less a scroll pulls.
const RESISTANCE: f32 = 0.35;
/// 停手后保持多久再回弹。
/// How long the pull holds after the wheel stops before springing back.
const HOLD: Duration = Duration::from_millis(90);
/// 滚动弹簧角频率：约 0.15 秒滑到目标。
/// Scroll spring angular frequency: reaches the target in about 0.15 s.
const SCROLL_OMEGA: f32 = 32.;
/// 回弹弹簧角频率：约 0.25 秒回到原位。
/// Pull spring angular frequency: returns in about a quarter second.
const PULL_OMEGA: f32 = 24.;
/// 单步积分上限，掉帧时拆成小步，保持稳定。
/// Largest integration step; a dropped frame is split into small steps to stay stable.
const STEP: f32 = 1. / 240.;

/// 临界阻尼弹簧：位置追随目标，不越过。
/// A critically damped spring: the position follows the target without overshooting.
#[derive(Default)]
struct Spring {
    target: f32,
    position: f32,
    velocity: f32,
}

impl Spring {
    fn advance(&mut self, omega: f32, mut left: f32) {
        while left > 0. {
            let dt = left.min(STEP);
            let pull = omega * omega * (self.target - self.position);
            self.velocity += (pull - 2. * omega * self.velocity) * dt;
            self.position += self.velocity * dt;
            left -= dt;
        }
    }

    fn settled(&self) -> bool {
        (self.position - self.target).abs() < 0.3 && self.velocity.abs() < 4.
    }

    fn snap(&mut self) {
        self.position = self.target;
        self.velocity = 0.;
    }
}

pub struct Elastic {
    /// 滚动位置，0 为顶端，向下为正。
    /// Scroll position: 0 at the top, positive downwards.
    scroll: Spring,
    /// 越界拉出量，正值内容向下。
    /// Pull past an edge; positive moves the content down.
    pull: Spring,
    moved: Instant,
    ticked: Instant,
    /// 内容与可视区高度，由界面在布局后写入。
    /// Content and viewport heights, written by the view after layout.
    pub content: Rc<Cell<f32>>,
    pub viewport: Rc<Cell<f32>>,
    /// 当前滚动所属的页面；换页时归零。
    /// The page this scroll belongs to; reset when the page changes.
    pub page: usize,
}

impl Default for Elastic {
    fn default() -> Self {
        let now = Instant::now();
        Self {
            scroll: Spring::default(),
            pull: Spring::default(),
            moved: now,
            ticked: now,
            content: Rc::default(),
            viewport: Rc::default(),
            page: 0,
        }
    }
}

impl Elastic {
    fn max(&self) -> f32 {
        (self.content.get() - self.viewport.get()).max(0.)
    }

    /// 处理一次滚轮。`dy` 为正表示向上滚。
    /// Takes one wheel step; positive `dy` scrolls up.
    pub fn wheel(&mut self, dy: f32) {
        if self.resting() {
            self.ticked = Instant::now();
        }
        let max = self.max();
        let at_top = self.scroll.target <= 0.;
        let at_bottom = self.scroll.target >= max;
        if (dy > 0. && at_top) || (dy < 0. && at_bottom) {
            // 已经停在边界还继续滚：拉出空白。
            // Already at a limit and still scrolling: pull out the blank.
            let room = (1. - self.pull.target.abs() / REACH).max(0.);
            self.pull.target = (self.pull.target + dy * RESISTANCE * room).clamp(-REACH, REACH);
            self.moved = Instant::now();
            return;
        }
        // 反向滚动先收回拉出的部分；这一下滚到边界就停在边界。
        // Scrolling back first takes up any pull; a scroll that reaches a limit stops there.
        self.pull.target = 0.;
        self.scroll.target = (self.scroll.target - dy).clamp(0., max);
    }

    /// 换页时回到顶端，不做动画。
    /// Back to the top without animation, for a page change.
    pub fn reset(&mut self) {
        self.scroll = Spring::default();
        self.pull = Spring::default();
    }

    fn resting(&self) -> bool {
        self.scroll.settled() && self.pull.settled() && self.pull.target == 0.
    }

    /// 推进两个弹簧，返回内容本帧的纵向位移和是否还在运动。
    /// Advances both springs; returns the content's vertical shift for this frame and whether it
    /// is still moving.
    pub fn frame(&mut self) -> (Pixels, bool) {
        let now = Instant::now();
        // 内容变矮（如折叠）时，目标收回到新的范围内。
        // When the content gets shorter (a collapse, say) the target moves back into range.
        self.scroll.target = self.scroll.target.clamp(0., self.max());
        if self.pull.target != 0. && now.duration_since(self.moved) >= HOLD {
            self.pull.target = 0.;
        }
        let elapsed = now.duration_since(self.ticked).as_secs_f32().min(0.1);
        self.ticked = now;
        self.scroll.advance(SCROLL_OMEGA, elapsed);
        self.pull.advance(PULL_OMEGA, elapsed);
        let moving = !self.resting();
        if !moving {
            self.scroll.snap();
            self.pull.snap();
        }
        (px(self.pull.position - self.scroll.position), moving)
    }

    /// 不做动画，直接取当前位移（系统开启“减少动画”时）。
    /// The shift without animation, for reduced motion.
    pub fn still(&mut self) -> Pixels {
        self.scroll.target = self.scroll.target.clamp(0., self.max());
        self.scroll.snap();
        self.pull = Spring::default();
        px(-self.scroll.position)
    }
}

#[cfg(test)]
#[path = "../../../test/frontend/desktop/ely_elastic.rs"]
mod tests;
