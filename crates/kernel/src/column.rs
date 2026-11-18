//! Typed storage: component columns and resources.

use std::any::Any;
use std::ops::{Index, IndexMut};

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::codec::{self, DecodeError, SeqReader};
use crate::ids::Idx;

/// A typed column of the resident table.
///
/// A component has one value per resident. The value moves with the resident
/// when it migrates to another zone, and it is part of the snapshot. `NAME`
/// identifies the component in snapshots; keep it stable.
pub trait Component: Serialize + DeserializeOwned + Default + Clone + Send + 'static {
    /// Unique name of the component: `<module>.<what>`.
    const NAME: &'static str;
}

/// A typed singleton in one realm, for example a market book.
///
/// A resource is part of the snapshot. `NAME` identifies it there; keep it
/// stable.
pub trait Resource: Serialize + DeserializeOwned + Default + Send + 'static {
    /// Unique name of the resource: `<module>.<what>`.
    const NAME: &'static str;
}

/// One component column: a value of `T` for every resident of the zone,
/// indexed by [`Idx`].
#[derive(Clone, Debug, Default)]
pub struct Column<T> {
    data: Vec<T>,
}

impl<T> Column<T> {
    /// Returns the number of residents.
    #[inline]
    #[must_use]
    pub fn len(&self) -> usize {
        self.data.len()
    }

    /// Returns true when the zone has no resident.
    #[inline]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// Returns the value of a resident, or `None` when the index is past the
    /// end.
    #[inline]
    #[must_use]
    pub fn get(&self, idx: Idx) -> Option<&T> {
        self.data.get(idx.index())
    }

    /// Returns the value of a resident for a change.
    #[inline]
    pub fn get_mut(&mut self, idx: Idx) -> Option<&mut T> {
        self.data.get_mut(idx.index())
    }

    /// Returns all values in index order.
    #[inline]
    #[must_use]
    pub fn as_slice(&self) -> &[T] {
        &self.data
    }

    /// Returns all values in index order for a change.
    #[inline]
    pub fn as_mut_slice(&mut self) -> &mut [T] {
        &mut self.data
    }

    /// Iterates over all values in index order.
    #[inline]
    pub fn iter(&self) -> std::slice::Iter<'_, T> {
        self.data.iter()
    }

    /// Iterates over all values in index order for a change.
    #[inline]
    pub fn iter_mut(&mut self) -> std::slice::IterMut<'_, T> {
        self.data.iter_mut()
    }
}

impl<T> Index<Idx> for Column<T> {
    type Output = T;

    #[inline]
    fn index(&self, idx: Idx) -> &T {
        &self.data[idx.index()]
    }
}

impl<T> IndexMut<Idx> for Column<T> {
    #[inline]
    fn index_mut(&mut self, idx: Idx) -> &mut T {
        &mut self.data[idx.index()]
    }
}

impl<'a, T> IntoIterator for &'a Column<T> {
    type Item = &'a T;
    type IntoIter = std::slice::Iter<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.data.iter()
    }
}

impl<'a, T> IntoIterator for &'a mut Column<T> {
    type Item = &'a mut T;
    type IntoIter = std::slice::IterMut<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.data.iter_mut()
    }
}

/// A component column whose type is erased, so that the resident table can
/// add, remove, move and save rows without knowing the type.
pub(crate) trait AnyColumn: Send {
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
    fn len(&self) -> usize;
    /// Adds a row with the default value.
    fn push_default(&mut self);
    /// Removes row `i` and moves the last row into its place.
    fn swap_remove(&mut self, i: usize);
    /// Appends row `i` as MessagePack to `out`.
    fn encode_row(&self, i: usize, out: &mut Vec<u8>);
    /// Adds a row that is read from `reader`.
    fn push_decoded(&mut self, reader: &mut SeqReader<'_>) -> Result<(), DecodeError>;
    /// Writes all rows as one MessagePack array.
    fn encode_all(&self) -> Vec<u8>;
    /// Replaces all rows with the rows of a MessagePack array.
    fn decode_all(&mut self, bytes: &[u8]) -> Result<(), DecodeError>;
    /// Makes the column `len` rows long. New rows have the default value.
    fn resize(&mut self, len: usize);
    /// Bytes of the rows, without what they point to.
    fn memory_estimate(&self) -> usize;
}

impl<T: Component> AnyColumn for Column<T> {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn len(&self) -> usize {
        self.data.len()
    }

    fn push_default(&mut self) {
        self.data.push(T::default());
    }

    fn swap_remove(&mut self, i: usize) {
        self.data.swap_remove(i);
    }

    fn encode_row(&self, i: usize, out: &mut Vec<u8>) {
        codec::encode_into(T::NAME, &self.data[i], out);
    }

    fn push_decoded(&mut self, reader: &mut SeqReader<'_>) -> Result<(), DecodeError> {
        let value = T::deserialize(reader).map_err(|e| DecodeError {
            what: format!("component `{}`", T::NAME),
            message: e.to_string(),
        })?;
        self.data.push(value);
        Ok(())
    }

    fn encode_all(&self) -> Vec<u8> {
        codec::encode(T::NAME, &self.data)
    }

    fn decode_all(&mut self, bytes: &[u8]) -> Result<(), DecodeError> {
        self.data = codec::decode(&format!("component `{}`", T::NAME), bytes)?;
        Ok(())
    }

    fn resize(&mut self, len: usize) {
        self.data.resize_with(len, T::default);
    }

    fn memory_estimate(&self) -> usize {
        self.data.capacity() * size_of::<T>()
    }
}

/// Makes an empty column of `T`.
pub(crate) fn new_column<T: Component>() -> Box<dyn AnyColumn> {
    Box::new(Column::<T> { data: Vec::new() })
}

/// A resource whose type is erased.
pub(crate) trait AnyResource: Send {
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
    fn encode(&self) -> Vec<u8>;
    fn decode(&mut self, bytes: &[u8]) -> Result<(), DecodeError>;
}

/// The box that holds one resource.
pub(crate) struct ResourceBox<T>(pub(crate) T);

impl<T: Resource> AnyResource for ResourceBox<T> {
    fn as_any(&self) -> &dyn Any {
        &self.0
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        &mut self.0
    }

    fn encode(&self) -> Vec<u8> {
        codec::encode(T::NAME, &self.0)
    }

    fn decode(&mut self, bytes: &[u8]) -> Result<(), DecodeError> {
        self.0 = codec::decode(&format!("resource `{}`", T::NAME), bytes)?;
        Ok(())
    }
}

/// Makes a resource of `T` with its default value.
pub(crate) fn new_resource<T: Resource>() -> Box<dyn AnyResource> {
    Box::new(ResourceBox(T::default()))
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;

    use super::*;

    #[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
    struct Needs {
        food: i32,
        note: String,
    }

    impl Component for Needs {
        const NAME: &'static str = "test.needs";
    }

    #[derive(Debug, Default, PartialEq, Serialize, Deserialize)]
    struct Book {
        orders: Vec<u32>,
    }

    impl Resource for Book {
        const NAME: &'static str = "test.book";
    }

    fn typed(column: &mut Box<dyn AnyColumn>) -> &mut Column<Needs> {
        column.as_any_mut().downcast_mut().unwrap()
    }

    #[test]
    fn rows_can_be_added_removed_and_indexed() {
        let mut column = new_column::<Needs>();
        column.push_default();
        column.push_default();
        column.push_default();
        typed(&mut column)[Idx(0)].food = 10;
        typed(&mut column)[Idx(2)].food = 30;
        column.swap_remove(0);
        assert_eq!(column.len(), 2);
        let col = typed(&mut column);
        assert_eq!(col[Idx(0)].food, 30);
        assert_eq!(col.get(Idx(1)).map(|n| n.food), Some(0));
        assert_eq!(col.get(Idx(2)), None);
        assert_eq!(col.iter().map(|n| n.food).sum::<i32>(), 30);
        for needs in col.iter_mut() {
            needs.food += 1;
        }
        assert_eq!(col.as_slice().iter().map(|n| n.food).collect::<Vec<_>>(), [31, 1]);
        assert!(!col.is_empty());
        column.resize(4);
        assert_eq!(column.len(), 4);
        assert!(column.memory_estimate() >= 4 * size_of::<Needs>());
    }

    #[test]
    fn a_row_moves_between_columns_as_bytes() {
        let mut from = new_column::<Needs>();
        from.push_default();
        *typed(&mut from).get_mut(Idx(0)).unwrap() = Needs { food: 7, note: "hungry".into() };
        let mut bytes = Vec::new();
        from.encode_row(0, &mut bytes);
        from.encode_row(0, &mut bytes);

        let mut to = new_column::<Needs>();
        let mut reader = codec::seq_reader(&bytes);
        to.push_decoded(&mut reader).unwrap();
        to.push_decoded(&mut reader).unwrap();
        assert_eq!(
            typed(&mut to).as_slice(),
            [Needs { food: 7, note: "hungry".into() }, Needs { food: 7, note: "hungry".into() }]
        );
        let error = to.push_decoded(&mut reader).unwrap_err();
        assert!(error.what.contains("test.needs"), "{error}");
        assert_eq!(to.len(), 2);
    }

    #[test]
    fn a_whole_column_and_a_resource_survive_serialization() {
        let mut column = new_column::<Needs>();
        column.push_default();
        column.push_default();
        typed(&mut column)[Idx(1)].note = "x".into();
        let bytes = column.encode_all();
        let mut back = new_column::<Needs>();
        back.decode_all(&bytes).unwrap();
        assert_eq!(typed(&mut back).as_slice(), typed(&mut column).as_slice());
        assert!(back.decode_all(&[0xc1]).is_err());

        let mut book = new_resource::<Book>();
        book.as_any_mut().downcast_mut::<Book>().unwrap().orders.push(5);
        let bytes = book.encode();
        let mut back = new_resource::<Book>();
        back.decode(&bytes).unwrap();
        assert_eq!(back.as_any().downcast_ref::<Book>(), Some(&Book { orders: vec![5] }));
        assert!(back.decode(&[0xc1]).is_err());
    }
}
