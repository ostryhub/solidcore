// Shared by the native ops tests and the isolated LTBL WASM provenance probe.
use super::solidcore_ops::{
    Body, BooleanOp,
    boolean::boolean_pair,
    provenance::{Entity, FIRST_ID, Provenance},
};

#[derive(Clone, Debug, PartialEq)]
struct Origins(Vec<usize>);

impl Provenance for Origins {
    const ID: u64 = FIRST_ID + 40;

    fn split(&self, _: Option<&Self>) -> Option<Self> {
        Some(self.clone())
    }

    fn merge(&self, other: Option<&Self>) -> Option<Self> {
        let mut origins = self.0.clone();
        if let Some(other) = other {
            origins.extend_from_slice(&other.0);
        }
        origins.sort_unstable();
        origins.dedup();
        Some(Self(origins))
    }

    fn coincident(&self, other: Option<&Self>) -> Option<Self> {
        self.merge(other)
    }
}

fn tag(body: &Body, base: usize) {
    let (_, order) = body.inspection_record().unwrap().unwrap();
    for i in 0..order.faces.len() {
        body.set_provenance(&order, Entity::Face, i, Origins(vec![base + i]))
            .unwrap();
    }
}

pub fn attach_read_and_copy() {
    let body = Body::seeded_box([0.0; 3], [3.0; 3]).unwrap();
    let (_, order) = body.inspection_record().unwrap().unwrap();
    assert_eq!(
        body.provenance::<Origins>(&order, Entity::Face, 0).unwrap(),
        None
    );
    body.set_provenance(&order, Entity::Face, 0, Origins(vec![8]))
        .unwrap();
    body.set_provenance(&order, Entity::Face, 0, Origins(vec![3]))
        .unwrap();
    body.set_provenance(&order, Entity::Edge, 0, Origins(vec![2]))
        .unwrap();
    assert_eq!(
        body.provenance(&order, Entity::Face, 0).unwrap(),
        Some(Origins(vec![3]))
    );
    assert_eq!(
        body.provenance(&order, Entity::Edge, 0).unwrap(),
        Some(Origins(vec![2]))
    );
    assert!(
        body.set_provenance(&order, Entity::Face, order.faces.len(), Origins(vec![7]))
            .is_err()
    );
    let copy = body.copy().unwrap();
    let (_, copy_order) = copy.inspection_record().unwrap().unwrap();
    assert_eq!(
        copy.provenance(&copy_order, Entity::Face, 0).unwrap(),
        Some(Origins(vec![3]))
    );
    assert_eq!(
        copy.provenance(&copy_order, Entity::Edge, 0).unwrap(),
        Some(Origins(vec![2]))
    );
}

pub fn slab_split() {
    let stock = Body::seeded_box([0.0; 3], [3.0; 3]).unwrap();
    tag(&stock, 0);
    let slab = Body::seeded_box([-1.0, -1.0, 1.0], [5.0, 5.0, 1.0]).unwrap();
    let split = boolean_pair(&stock, &slab, BooleanOp::Subtract, true).unwrap();
    let (_, order) = split.inspection_record().unwrap().unwrap();
    assert_eq!(order.faces.len(), 12);
    let mut counts = [0; 6];
    let mut untagged = 0;
    for i in 0..order.faces.len() {
        match split
            .provenance::<Origins>(&order, Entity::Face, i)
            .unwrap()
        {
            Some(Origins(origins)) => {
                assert_eq!(origins.len(), 1);
                counts[origins[0]] += 1;
            }
            None => untagged += 1,
        }
    }
    counts.sort_unstable();
    assert_eq!(counts, [1, 1, 2, 2, 2, 2]);
    assert_eq!(untagged, 2);
    assert_eq!(stock.counts().unwrap(), [6, 12, 8]);
    assert!((stock.volume_without_attributes(1e-9).unwrap() - 27.0).abs() <= 1e-9);
    assert!((split.volume_without_attributes(1e-9).unwrap() - 18.0).abs() <= 1e-9);
}

pub fn coplanar_merge() {
    let left = Body::seeded_box([0.0; 3], [1.0; 3]).unwrap();
    let right = Body::seeded_box([1.0, 0.0, 0.0], [1.0; 3]).unwrap();
    tag(&left, 0);
    tag(&right, 100);
    let joined = boolean_pair(&left, &right, BooleanOp::Union, true).unwrap();
    let (_, order) = joined.inspection_record().unwrap().unwrap();
    assert_eq!(joined.counts().unwrap(), [6, 12, 8]);
    let top = (0..order.faces.len())
        .find(|&i| {
            let (lo, hi) = joined
                .bounds_for_order(&order, &[i], &[], &[])
                .unwrap()
                .unwrap();
            (lo[2] - 1.0).abs() < 1e-9 && (hi[2] - 1.0).abs() < 1e-9
        })
        .unwrap();
    let Origins(origins) = joined
        .provenance::<Origins>(&order, Entity::Face, top)
        .unwrap()
        .unwrap();
    assert_eq!(origins.len(), 2);
    assert!(origins[0] < 6);
    assert!((100..106).contains(&origins[1]));
    assert_eq!(left.counts().unwrap(), [6, 12, 8]);
    assert_eq!(right.counts().unwrap(), [6, 12, 8]);
}
