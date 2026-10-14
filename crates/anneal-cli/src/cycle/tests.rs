use super::find_cycle;

#[test]
fn an_acyclic_graph_has_no_cycle() {
    let dependencies: [&[usize]; 4] = [&[], &[0], &[0, 1], &[2]];

    assert_eq!(find_cycle(&dependencies), None, "a chain and a diamond");
}

#[test]
fn an_empty_graph_has_no_cycle() {
    let dependencies: [Vec<usize>; 0] = [];

    assert_eq!(find_cycle(&dependencies), None, "nothing to walk");
}

#[test]
fn a_node_depending_on_itself_is_a_cycle_of_one() {
    let dependencies: [&[usize]; 2] = [&[], &[1]];

    assert_eq!(find_cycle(&dependencies), Some(vec![1]), "self-loop");
}

#[test]
fn a_cycle_is_rotated_to_start_at_its_smallest_index() {
    // 3 -> 1 -> 2 -> 3, entered from 0 at 3.
    let dependencies: [&[usize]; 4] = [&[3], &[2], &[3], &[1]];

    assert_eq!(
        find_cycle(&dependencies),
        Some(vec![1, 2, 3]),
        "each depends on the next, the last on the first; node 0 only waits on the cycle"
    );
}

#[test]
fn dependencies_are_followed_in_the_order_given() {
    // 0 depends on 1 and 2; both 1 and 2 close a loop back to 0.
    let dependencies: [&[usize]; 3] = [&[2, 1], &[0], &[0]];

    assert_eq!(
        find_cycle(&dependencies),
        Some(vec![0, 2]),
        "the first dependency listed is walked first"
    );
}

#[test]
fn an_index_out_of_range_is_ignored() {
    let dependencies: [&[usize]; 2] = [&[7], &[0, 99]];

    assert_eq!(find_cycle(&dependencies), None, "no node 7 or 99");
}

#[test]
fn a_long_chain_is_walked_without_recursion() {
    let length = 100_000_usize;
    let mut dependencies: Vec<Vec<usize>> = (0..length)
        .map(|node| node.checked_sub(1).into_iter().collect())
        .collect();

    assert_eq!(find_cycle(&dependencies), None, "a deep chain is fine");

    dependencies
        .first_mut()
        .expect("non-empty")
        .push(length - 1);
    let cycle = find_cycle(&dependencies).expect("closed into a ring");
    assert_eq!(cycle.len(), length, "the whole ring");
    assert_eq!(cycle.first(), Some(&0), "from the smallest index");
}
