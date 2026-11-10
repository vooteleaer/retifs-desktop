//! What the tray icon says, on every platform: whether retifs and
//! Reticulum are up and others reachable, its lines of text, and the icon
//! itself drawn in code (no image files to ship).

/// What the icon shows.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TrayState {
    pub retifs_up: bool,
    pub node: Option<String>,
    /// While retifs starts: what it's doing.
    pub starting: Option<String>,
    pub reticulum_up: bool,
    /// Connections to other nodes that are up, of those configured.
    pub links_up: usize,
    pub links: usize,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Health {
    /// retifs and Reticulum up, others reachable.
    Good,
    /// retifs up, but no connection to others is.
    Partly,
    /// retifs isn't running.
    Down,
}

impl TrayState {
    /// From the daemon's status and Reticulum's: links are the connections
    /// to other nodes (the shared instance's local ones don't count).
    pub fn of(status: &crate::Status, r: &crate::device::Reticulum) -> TrayState {
        let outward: Vec<_> = r.interfaces.iter().filter(|i| !i.is_local()).collect();
        TrayState {
            retifs_up: status.up,
            node: status.node.clone(),
            starting: status.starting_text(),
            reticulum_up: r.running,
            links_up: outward.iter().filter(|i| i.up).count(),
            links: outward.len(),
        }
    }

    pub fn health(&self) -> Health {
        match (self.retifs_up, self.reticulum_up) {
            (true, true) if self.links_up > 0 || self.links == 0 => Health::Good,
            (true, _) => Health::Partly,
            (false, _) => Health::Down,
        }
    }

    pub fn lines(&self) -> Vec<String> {
        vec![
            match &self.node {
                Some(n) if self.retifs_up => format!("retifs: running, node {}…", &n[..8.min(n.len())]),
                _ => match &self.starting {
                    Some(step) => format!("retifs: starting ({step})"),
                    None => "retifs: not running".to_string(),
                },
            },
            if self.reticulum_up { format!("Reticulum: running, {} of {} connection(s) up", self.links_up, self.links) } else { "Reticulum: no shared instance".to_string() },
        ]
    }
}

/// The icon at `size` pixels, as ARGB (one u32 per pixel, alpha in the top
/// byte): the retifs mark (a hub and four nodes), green when all is well,
/// amber when no connection is up, grey when retifs isn't running.
pub fn draw(size: u32, health: Health) -> Vec<u32> {
    let rgb: u32 = match health {
        Health::Good => 0x5fb06e,
        Health::Partly => 0xe0a030,
        Health::Down => 0x909090,
    };
    let s = size as f32;
    let c = s / 2.0;
    let unit = s / 32.0;
    let mut out = Vec::with_capacity((size * size) as usize);
    for y in 0..size {
        for x in 0..size {
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            let d = ((px - c).powi(2) + (py - c).powi(2)).sqrt();
            let ring = (d - 5.0 * unit).abs() < 1.6 * unit;
            let spoke = ((px - c).abs() < 1.2 * unit || (py - c).abs() < 1.2 * unit) && d > 6.5 * unit && d < 11.5 * unit;
            let node = [(c, 3.5 * unit), (c, s - 3.5 * unit), (3.5 * unit, c), (s - 3.5 * unit, c)]
                .iter()
                .any(|(nx, ny)| ((px - nx).powi(2) + (py - ny).powi(2)).sqrt() < 3.4 * unit);
            out.push(if ring || spoke || node { 0xff00_0000 | rgb } else { 0 });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn health_and_lines() {
        let mut s = TrayState { retifs_up: true, node: Some("1b95a9e9cafe".into()), starting: None, reticulum_up: true, links_up: 0, links: 2 };
        assert_eq!(s.health(), Health::Partly, "up but nobody reachable");
        s.links_up = 1;
        assert_eq!(s.health(), Health::Good);
        assert_eq!(s.lines(), vec!["retifs: running, node 1b95a9e9…", "Reticulum: running, 1 of 2 connection(s) up"]);
        assert_eq!(TrayState::default().health(), Health::Down);
        let icon = draw(32, Health::Good);
        assert_eq!(icon.len(), 32 * 32);
        assert!(icon.contains(&0xff5fb06e) && icon.contains(&0), "drawn on transparent");
    }
}
