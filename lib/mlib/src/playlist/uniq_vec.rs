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

    pub fn remove_at(&mut self, i: usize) -> T {
        self.v.remove(i)
    }

    pub fn into_vec(self) -> Vec<T> {
        self.v
    }

    pub fn iter(&self) -> std::slice::Iter<'_, T> {
        self.v.iter()
    }

    pub fn iter_mut(&mut self) -> std::slice::IterMut<'_, T> {
        self.v.iter_mut()
    }

    pub fn retain(&mut self, f: impl FnMut(&mut T) -> bool) {
        self.v.retain_mut(f)
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
        let mut v = Vec::new();
        for i in iter {
            v.push(i)
        }
        Self { v }
    }
}

impl<T> From<Vec<T>> for UniqVec<T>
where
    T: PartialEq,
{
    fn from(value: Vec<T>) -> Self {
        for i in 0..value.len() {
            for j in (i + 1)..value.len() {
                if value[i] == value[j] {
                    return Self::from_iter(value);
                }
            }
        }
        Self { v: value }
    }
}

impl<T> IntoIterator for UniqVec<T> {
    type Item = T;
    type IntoIter = std::vec::IntoIter<T>;

    fn into_iter(self) -> Self::IntoIter {
        self.v.into_iter()
    }
}

impl<'s, T> IntoIterator for &'s UniqVec<T> {
    type Item = &'s T;
    type IntoIter = std::slice::Iter<'s, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<'s, T> IntoIterator for &'s mut UniqVec<T> {
    type Item = &'s mut T;
    type IntoIter = std::slice::IterMut<'s, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.v.iter_mut()
    }
}
