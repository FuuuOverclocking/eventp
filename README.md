# eventp

## 草稿

分成几种情况：
- own/&mut -> AsMut
- T/dyn -> AsThinPtrMut

Sized: AsMut<T>
- own T
- &mut T

!Sized: AsThinPtrMut<DynSub>
- Box<T>
- &mut DynSub
