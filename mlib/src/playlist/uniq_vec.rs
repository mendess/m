use std::ops::Deref;

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Clone)]
#[serde(transparent)]
pub struct UniqVec<T> {
    v: Vec<T>,
}

impl<T> Default for UniqVec<T> {
    fn default() -> Self {
        Self {
            v: Default::default(),
        }
    }
}

impl<T: PartialEq> UniqVec<T> {
    #[inline(always)]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, t: T) -> Option<T> {
        if self.v.contains(&t) {
            Some(t)
        } else {
            self.v.push(t);
            None
        }
    }

    pub fn remove(&mut self, t: &T) -> bool {
        if let Some(i) = self.v.iter().position(|e| e == t) {
            self.v.remove(i);
            true
        } else {
            false
        }
    }

    pub fn into_vec(self) -> Vec<T> {
        self.v
    }
}

impl<T> Deref for UniqVec<T> {
    type Target = [T];

    fn deref(&self) -> &Self::Target {
        &self.v
    }
}

impl<T> FromIterator<T> for UniqVec<T> {
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        Self {
            v: FromIterator::from_iter(iter),
        }
    }
}
