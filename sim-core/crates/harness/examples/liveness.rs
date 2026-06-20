fn main() {
    let sc = harness::m0_scenario();
    let mut s = sim::ArenaState::new(sc.master_seed, sc.player_id);
    let (mut max_enemies, mut min_hp, mut max_entity) = (0usize, s.tank.hp, 0u32);
    for tick in 0..sc.total_ticks {
        let inp = sc.scripted.iter().find(|(t,_)| *t==tick).map(|(_,i)|*i).unwrap_or(sim::Input::Noop);
        sim::step(&mut s, inp);
        max_enemies = max_enemies.max(s.enemies.len());
        min_hp = min_hp.min(s.tank.hp);
        max_entity = max_entity.max(s.next_entity_id);
    }
    println!("final: gold={} tank_hp={}/{} weapons={} enemies_alive={}", s.economy.gold, s.tank.hp, s.tank.max_hp, s.weapons.len(), s.enemies.len());
    println!("over run: peak_enemies={} min_tank_hp={} entities_created={}", max_enemies, min_hp, max_entity);
}
