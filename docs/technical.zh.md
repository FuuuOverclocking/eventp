# Technical

[English](crate::_technical) | 中文

`Eventp` 提供了一套零开销的事件分发机制, 同时还有简洁、易于测试的 API.
本文将一步步讲述, 这两件事是如何同时做到的.

---
## 1. 从 mio 到 event-manager, 再到 eventp

[mio](https://docs.rs/mio/latest/mio/) 是 `epoll`/`kqueue`/IOCP 的一层薄薄的跨平台封装.
你给它一组 fd, 它告诉你哪些就绪, 你再用 `match` 去匹配 `Token` (一个用户自选的 `usize`)
来决定接下来要做什么. 本质上就是"带跨平台的裸 `epoll`" —— 
[mio 的 tcp_server 示例](https://github.com/tokio-rs/mio/blob/master/examples/tcp_server.rs).

[event-manager](https://docs.rs/event-manager/latest/event_manager/) 在此之上多走了一步:
它引入了一层*订阅*抽象. 每个 fd 都属于一个 `Subscriber` 对象, 由它自己处理事件;
事件分发在运行时可以修改, 处理函数内部还能注册新的事件源. 这种编程模型对大型项目 (比如 rust-vmm)
相对友好得多 -- 
[basic example](https://github.com/rust-vmm/event-manager?tab=readme-ov-file#basic-single-thread-subscriber).

到这里都很好. 但代价随之而来.

### 1.1 代价: 每个事件三次 HashMap 查询

当一个 `Subscriber` 的处理函数被触发时, 它通常想做两件事:

1. 读写自己的数据 (`&mut self`).
2. 操作 reactor —— 添加新 fd、注销自己、修改 interest (`&mut Reactor`).

但这两个 `&mut` 是重叠的, 因为 `Subscriber` 本身就是 `Reactor` 的一部分. 借用检查器自然不答应.
直接的绕路办法, 就是放弃把它们放在一起, 把所有权重新洗一遍. event-manager 正是这么做的,
它使用了 **4 个** `HashMap` 拼成的三层结构:

![event-manager](https://raw.githubusercontent.com/FuuuOverclocking/eventp/refs/heads/main/docs/images/event-manager.svg)

这下两个 `&mut` 来自真正不同的对象, 借用检查器满意了. 代价是, 每个事件的分发要经过三次 `HashMap` 查询.

更糟的是, 这里用的是 `std::collections::HashMap`, 它默认的哈希算法是 SipHash 1-3,
一个抗 HashDoS 的算法 —— 这对 HTTP header 来说无可挑剔, 但我们的 key 是*内核分配出来的小整数 fd*,
根本就没有所谓的攻击者. 我们花着钱, 给一个不存在的威胁穿上了铠甲.

### 1.2 隐藏的炸弹: fd 复用引发的"幽灵事件"

依据 `RawFd` 做事件分发, **很容易**踩到一类 ABA bug. POSIX 明确规定:
[`open(2)`](https://man.archlinux.org/man/open.2.en)、`accept(2)`、`socket(2)`、`pipe(2)` 这些调用,
必须返回**当前进程中数值最小的、未被占用的 fd**. 这意味着, 一个 fd 一旦关闭, 它的整数值会立刻成为
下一次开 fd 的*第一候选*. 复用是常态, 而不是例外.

考虑这样一段时序, 这正是以 fd 为主键的分发表容易招来的:

1. subscriber `A` 持有 `fd = 7`, 已经注册在 reactor 里, 分发表中存在一条 key 为 `7` 的记录.
2. `A` 的析构 (或它触发的某条更深的析构链) 关掉了 `fd = 7`, 但忘了 (或来不及) 注销.
3. 进程稍后 `accept` 出一个新连接, 内核回收 `7` 作为它的 fd, 应用又把它注册成 subscriber `B`.
4. epoll 触发, reactor 拿 `fd = 7` 查表, 命中的却是 `A` 的条目 —— **事件被分发到一具尸体上**.

这一类"幽灵事件" Bug 的可怕之处在于:

- **静默无声**. 编译器看不见, 单元测试几乎复现不出, 只有在生产环境的繁忙时段, 配合一份事故报告才会浮出水面.
- **跨越所有权边界**. 即使 `A` 的内存早就被回收并被别的对象重用, 那条陈旧的 `RawFd → subscriber id`
  映射依然在那里. 事件会被路由到那块内存现在的"住户"身上 —— 祝你好运.
- **本质上不是用户的错**. API 的*形状*在引诱用户写出"先 close, 再 remove"的顺序, 尤其当 `close`
  发生在 `Drop` 链深处时更是如此. 把这种细节甩给用户去保证, 是设计上的失败.

### 1.3 eventp 的关键洞察

`epoll_ctl(2)` 允许你给每个注册的 fd 附带一个任意的 8 字节负载 (`epoll_data_t`).
事件触发时, `epoll_wait(2)` 会把同一份负载原样还给你. 从语义上, 它就是个自由的"上下文指针"槽 ——
事实上 man page 推荐的用法本身就是这样.

那么思路就清楚了: **把处理函数对象在堆上的地址塞进去**. 事件触发, 我们把 `u64` 重新解释为指针,
做一次虚函数调用, 直接进入用户代码. 不查 hash, 不查表, 一条 `callq` 解决战斗.

这同时也彻底端掉了"幽灵事件"这一类问题: 路由现在跟随的是内核回交的对象指针, 而不是 `RawFd` 查表.
fd 整数被复用与否完全无关 —— 不同的 fd 意味着不同的注册, 也就是不同的指针. 而且
`Eventp::delete` 的实现保证了 subscriber 释放和 `EPOLL_CTL_DEL` 是绑死的, 也就是说 API 根本
不再暴露"忘记 remove"这条路径.

当然, 天下没有免费的午餐. 要让这套思路在 Rust 里跑起来, 我们要解决三个 Rust 特有的难题, 而本文剩下的部分,
就是这三件事的故事:

1. `&dyn Trait` 在 64 位平台上是 16 字节, 塞不进 `u64`.
2. 把 `&mut Reactor` 交给一个本身住在 reactor 里的 handler —— 这是教科书级别的双重可变借用.
3. handler 可能在批量事件分发的过程中改动 reactor (添加、修改、删除, 甚至删除自己),
   我们必须保证这一切是 sound 的.


---

## 2. 给胖指针瘦身: `ThinBoxSubscriber`

### 2.1 为什么必须是运行时多态

你也许会问: 为什么不直接让 reactor 对 `T: Subscriber` 做泛型, 让单态化把活都干了? 实际项目里
—— VMM 这种场景尤其明显 —— 大约 90% 的真实 reactor, 持有的 subscriber 是*许多*不同的具体类型:
一个控制 eventfd, 一个 TCP listener, 一堆 TCP 连接, 一个串口控制台, 一个 vsock 通道……
泛型参数一旦出现在 reactor 类型上, 就会病毒式地一路传染到 `fn main`. trait object 才是务实的答案,
代价是一次间接调用, 我们认.

但 Rust 的 trait object 表示带来了麻烦:

<figure style="display: inline-block;">
<img src="https://raw.githubusercontent.com/FuuuOverclocking/eventp/refs/heads/main/docs/images/pointer-meta.svg" alt="Rust 胖指针" />
<figcaption style="text-align: center;">Rust 胖指针</figcaption>
</figure>

<figure style="display: inline-block;">
<img src="https://raw.githubusercontent.com/FuuuOverclocking/eventp/refs/heads/main/docs/images/cpp-vptr.svg" alt="C++ 单继承对象指针" />
<figcaption style="text-align: center;">C++ 单继承对象指针</figcaption>
</figure>

Rust 的 `&dyn Trait` 是一个**胖指针**: 数据指针 + vtable 指针, 在 64 位平台上是 16 字节,
比 `epoll_data_t` 多了整整 8 个字节.

### 2.2 关键洞察: 别怕分配器

`rustc` 给的内存布局是默认值, 不是牢笼. 既然我们能自己管分配, 那就没什么能阻止我们把 vtable 指针
**塞进**对象内部, 仿照 C++ 的做法. 这样指向对象的指针就只有一个 word —— 而这个 word, 正好可以塞进
`epoll_event.data`.

### 2.3 起手式

我们一步步搭起来.

```rust,ignore
pub struct ThinBoxSubscriber {
    ptr: NonNull<u8>,
    _marker: PhantomData<dyn Subscriber>,
}

impl ThinBoxSubscriber {
    pub fn new<T: Subscriber>(value: T) -> Self {
        todo!()
    }
}
```

#### Step 1: 先把例外排掉

我们只支持 64 位 Linux, 其余一律编译报错:

```rust,ignore
#[cfg(not(target_pointer_width = "64"))]
compile_error!("Platforms with pointer width other than 64 are not supported.");
```

这下我们能在*编译期*钉死一个事实:

```rust,ignore
const _: () = assert!(size_of::<&dyn Subscriber>() == 16);
```

未来某个工具链如果改了 trait object 的布局, 编译当场就会失败 —— 不会有静默 miscompile.

#### Step 2: 从胖指针里抠出 vtable

胖指针在内存里*就是*一个 `(data, vtable)` 对. 直接 `transmute`:

```rust,ignore
let fat_ptr = &value as &dyn Subscriber;
let (_data_ptr, vptr) = unsafe {
    mem::transmute::<&dyn Subscriber, (*const (), *const ())>(fat_ptr)
};
```

接下来, 我们想要一个堆上布局, 它以 vptr 开头:

<figure style="display: inline-block;">
<img src="https://raw.githubusercontent.com/FuuuOverclocking/eventp/refs/heads/main/docs/images/step-2.svg" alt="初始布局: vptr 后跟 T" />
<figcaption style="text-align: center;">第一版尝试: <code>(vptr, T)</code></figcaption>
</figure>

**微小但致命: align 空隙**. 如果 `T` 的对齐要求大于 `usize` (比如 `#[repr(align(16))]`,
或者结构体里塞了个 `__m128`), 编译器会在 `vptr` 和 `value` 之间偷偷塞 padding:

![step-2-align-issue](https://raw.githubusercontent.com/FuuuOverclocking/eventp/refs/heads/main/docs/images/step-2-align-issue.svg)

也就是说, `value` 不在 `ptr + size_of::<usize>()` 的位置. Deref 时算错地址, 直接 UB.

**小技巧: 让 `vptr` 紧贴 `value`, padding 落在外面**. 用 [`Layout::extend`] 把"一个 `usize`
作为 header (用来放 vtable 指针)"和"`T` 自己的 layout"组合起来. 分配器顺手会告诉我们 `T` 的偏移,
而 padding 会被塞在 *header 之前*, 而不是 header 和 `T` 之间:

[`Layout::extend`]: core::alloc::Layout::extend

```rust,ignore
let (layout, value_offset) = Layout::new::<usize>()
    .extend(Layout::new::<T>())
    .expect("Failed to create combined layout");
```

接着我们让 `ptr` 直接指向 `T`, 而 `vptr` 一定可以在固定的负偏移 `ptr - 8` 处读到.

> 思考: 为什么 vptr 落在那个位置一定合法? (提示: repr C 的对齐规则)

![step-2-align-issue-solved](https://raw.githubusercontent.com/FuuuOverclocking/eventp/refs/heads/main/docs/images/step-2-align-issue-solved.svg)

#### Step 3: 分配、放置、取地址

```rust,ignore
let ptr = unsafe {
    let raw = alloc::alloc(layout);
    if raw.is_null() { alloc::handle_alloc_error(layout); }
    NonNull::new_unchecked(raw.add(value_offset))   // 指向 T, 而不是分配起始
};
unsafe {
    ptr.as_ptr().sub(size_of::<usize>())            // vptr 槽位
       .cast::<*const ()>().write(vptr);
    ptr.as_ptr().cast::<T>().write(value);          // 把 T move 进去
}
```

`Deref` 就是反过来 —— 从 `ptr - 8` 读出 vptr, 跟 `ptr` 拼成胖指针,
还给调用者一个 `&mut dyn Subscriber<Ep>`.

### 2.4 Drop, 还要 panic-safe

Drop 这一步才有意思. 我们要做两件事:

1. 跑 `T` 的析构.
2. `dealloc` 这块堆.

那万一第 1 步 panic 了呢? 根据
[panic-in-drop 那场讨论](https://github.com/Amanieu/rfcs/blob/panic-in-drop/text/0000-panic-in-drop.md)
(RFC 已撤回, 但行为没变), `Drop` 里 panic 会触发 unwind. 如果我们天真地写成
`drop_in_place(value); dealloc(ptr)`, 第 1 步 unwind 时第 2 步会被跳过 —— 内存泄漏当场上演.

老办法是经典的*"在 Drop 里再放一个 Drop"*: 把"释放堆空间"的责任交给一个本地 guard struct,
它的 `Drop` 是无条件执行的:

```rust,ignore
let _guard = DropGuard { ptr, value_layout, _marker: PhantomData };
unsafe { ptr::drop_in_place(value_ptr) };  // 可能 panic
// _guard.drop() 无论走哪条路径都会运行, 调用 alloc::dealloc.
```

这个模式在 [`Vec`] 和大多数 RAII 容器里到处都是 —— 不过这次, 我们刚好身处少数需要亲自把它写一遍的场景.

[`Vec`]: std::vec::Vec

### 2.5 与真实代码的差距

实际的 [src/thin.rs](https://github.com/FuuuOverclocking/eventp/blob/main/src/thin.rs)
比上面更花哨一点点:

- **header 里还顺带塞了 `raw_fd`** (紧挨着 `vptr`). 这能省掉一些 `as_fd()` 的虚函数调用.
  它还兼任哨兵: 值为 -1 时, 表示 `value` 已经 `drop_in_place` 过了, 但堆空间本身还没回收.
  §4 会用到这点.
- **`Subscriber<Ep>` 对 reactor 类型是泛型的** (这样 mock 版的 reactor 也能塞进同一个
  `ThinBoxSubscriber<MockEventp>`). 纯粹的形式上的改动, 本身没什么意思.
- **`from_box_dyn`** 让你能把一个*已经类型擦除过的* `Box<dyn Subscriber<Ep>>` 转换成
  `ThinBoxSubscriber`.

### 2.6 顺带抹掉 fd 复用 Bug

现在的事件路由:

```text
epoll_wait → ev.data() (u64) → 重解释为 &mut dyn Subscriber<Ep>
```

整条分发路径上没有 `RawFd → subscriber` 这张表. 内核还回来的就是当年注册时给它的那个堆地址,
所以"幽灵 subscriber 收到事件"的唯一可能, 是堆空间被在 epoll 背后偷偷释放了 ——
而唯一能注销 subscriber 的 API (`Eventp::delete`), 同时也是唯一调用 `EPOLL_CTL_DEL` 的入口.
两件事被焊在一起, 你不可能只做一边.

---

## 3. 双重可变借用问题

### 3.1 我们最想写的接口

理想中的用户代码长这样, 直白得不能再直白:

```rust,ignore
trait Subscriber {
    fn handle(&mut self, reactor: &mut Eventp);
}
```

然而借用检查器不这么想:

```text
error[E0499]: cannot borrow `*reactor` as mutable more than once at a time
```

……因为 `*self` *住在* `reactor.registered` 里面, 你刚才同时申请了两个互相重叠的 `&mut`.
event-manager 的应对方式, 就是 §1 提到的三层 HashMap, 代价是每个事件三次查询. 这账, 我们不太想付.

### 3.2 换个角度想

把视角反过来. 假设我们就只用一个 map:

```rust,ignore
use rustc_hash::FxHashMap;  // 高性能哈希, 不抗 DoS (我们也不需要)

struct Eventp {
    registered: FxHashMap<RawFd, ThinBoxSubscriber>,
    // ...
}
```

再假设我们接受这样一种"逻辑上把 `&mut Eventp` 拆成两半"的视角:

- `&mut subscriber_i` —— 当前正在分发的那个
- `&mut (Eventp − subscriber_i)` —— 其余一切

由 §2 我们已经知道, `ThinBoxSubscriber` 不过是一个指针. 真正的 subscriber 数据躺在*另外一块*堆分配里,
map 只是引用了它. 因此, 当我们从 `self.registered[fd].deref()` 拿出 `&mut subscriber`
交给 `Subscriber::handle` 时, 唯一能让这个引用失效的事情, 就是有人把那块堆释放或搬走.

那么在 handler 调用期间, 一个 `&mut Eventp` 究竟*能*对那块堆做什么? 三件事:

1. **公开字段访问** (`reactor.registered = ...`). 好办: 一个字段都不开 `pub`.
2. **公开方法调用** (`reactor.some_method(&mut self)`). 烦, 但*我们*控制方法集合, 不暴露危险方法即可.
3. **`mem::replace`、`mem::take`、`*reactor = new_reactor`**. 💥 旧的 `Eventp` 当场原地析构,
   连带着整个 `registered` map, 当然也连带着我们正待在里面的那块堆. 此时 handler 手里的 `&mut self`
   突然指向了已被释放的内存.

第 1、2 类我们能管. 第 3 类才是真正的拦路虎.

### 3.3 黑魔法再向前一步: `Pin`

我们需要一种办法, 给 handler 一份"看起来像 `&mut Eventp`、**但第 3 类能力被外科切除**"的东西.
所幸, Rust 已经走过这条路了. 当年 async/await 在设计时, [`Future`] 撞上的是同一道险滩 —— `async fn`
返回的 `Future` 本身是个自指的状态机, `mem::replace` 它会让它内部的指针失效. 经过漫长的讨论和漫长的文档,
最终的答案是 [`Pin`].

[`Future`]: core::future::Future
[`Pin`]: core::pin

抛开 [Pin 那十六章劝退级文档](core::pin) 不谈, 它对我们真正要紧的只有一件事:
safe 代码**无法**把 `Pin<&mut T>` 还原为 `&mut T`, 除非 `T: Unpin`. 类型自己写的 inherent method
当然可以用 `unsafe` 内部投影回 `&mut T`, 但这些方法是类型作者写的, 可以选择永远不把值搬出来.

那思路就清楚了: 给 `Eventp` 标 `!Unpin` (一个 `PhantomPinned` 字段就够), 然后给 handler 一个
`Pin<&mut Eventp>`. 第 3 类问题消失了. safe 用户代码*没办法* `mem::replace` 掉 reactor.

```rust,ignore
struct Eventp {
    registered: FxHashMap<RawFd, ThinBoxSubscriber>,
    _pinned: PhantomPinned,
    // ...
}

trait Subscriber {
    fn handle(&mut self, reactor: Pin<&mut Eventp>);
    //                            ^^^^^^^^^^^^^^^^
    //              "你可以用它, 但你没办法让它消失"
}
```

不过别着急欢呼, 把 [The Problem With Single-threaded Shared
Mutability](https://manishearth.github.io/blog/2015/05/17/the-problem-with-shared-mutability/)
牢记在心, 这是我们的归途. 真正让这一切安全的不是 `Pin` 挥了挥魔杖, 而是我们*精心控制*的、
开放在被 pin 住的 reactor 上的方法集合 —— 我们会刻意把它收得很窄.

### 3.4 [`Pinned<'_, Ep>`](crate::Pinned): 一个故意做窄的 API

与其直接把 `Pin<&mut Eventp>` 交出去 (那以后我们给 `Pin<&mut Eventp>` 加的任何 inherent method,
用户都能调到), 不如用一个 newtype 把它包起来, 上面*只*开三个方法, 与 `epoll_ctl(2)` 一一对应:

```rust,ignore
pub struct Pinned<'a, Ep>(pub Pin<&'a mut Ep>);

impl<'a, Ep: EventpOps> Pinned<'a, Ep> {
    pub fn add(&mut self, sub: ThinBoxSubscriber<Ep>) -> io::Result<()> { ... }
    pub fn modify(&mut self, fd: RawFd, interest: Interest) -> io::Result<()> { ... }
    pub fn delete(&mut self, fd: RawFd) -> io::Result<()> { ... }
}
```

正好就是 `EPOLL_CTL_*` 的三个操作, 不多一个. 什么 `run_once`、`into_inner`、`Drop`、`Default` ——
在 handler 里通通够不着. reactor 不能被搬走, 不能被替换, 甚至不能再次进入 `epoll_wait`.
"handler 能对 reactor 做什么"的爆炸半径, 由构造确定就是三个系统调用的爆炸半径.

### 3.5 顺便澄清一下 `!Unpin` 到底保证了什么

一个容易看走眼的细节: `!Unpin` **不**保证 `registered` map "在内存中不动" —— `FxHashMap`
照样会在 `add` 新 subscriber 时该 rehash 就 rehash, 该洗桶就洗桶. `!Unpin` 保证的是
*`Eventp` 这个结构体本身*不能被搬走或替换, 所以它的 `registered` *字段*不会被人从底下抽走.

那么, 为什么 rehash 期间, 那个正在跑的 `&mut Subscriber` 不会失效呢? 答案是 §2 的间接性:
map 里只存 `ThinBoxSubscriber` (一个 word 的 handle), *subscriber 数据本身在另一块堆上*.
rehash 搬动的是这个一字 handle, 不是它指向的字节. 因此 handler 手里的 `&mut self` 仍指向同一个堆地址.

换句话说: §2 和 §3 是配合工作的. 瘦指针给我们提供了"rehash 中指针不变"的稳定性, `Pin` 给我们提供了
"对抗 `mem::replace`"的稳定性. 缺一不可.

---

## 4. handler 内部: 重入与 `Handling` 状态机

§3 解释了为什么把 `&mut Eventp` (以收窄的形式) 交出去是安全的, 但留下了一个更难的问题: handler
**实际拿到这只受限的句柄之后**, 它能做什么、不能做什么, 才不至于让正在分发的那个 subscriber 引用失效?

### 4.1 逐操作风险分析

`epoll_wait` 一次最多返回 N 个就绪事件, 我们逐个分发. handler `i` 跑的时候, 它可能反过来调 reactor.
对每种操作, 我们都要回答一个问题: 这是否会破坏当前的分发循环?

| handler 内部的操作            | 风险点                                                                              | 结论                                          |
| ----------------------------- | ----------------------------------------------------------------------------------- | --------------------------------------------- |
| `add(new_sub)`                | `FxHashMap` rehash. 但瘦指针稳定, 而且新 sub 不在本批次内.                          | 放行.                                         |
| `modify(other, ..)`           | 改内核状态 + sub 内部的 `Cell<Interest>`. 不动其他东西.                             | 放行.                                         |
| `delete(other)`               | `other` 的事件可能也在本批次中 —— 直接 `dealloc` ⇒ 悬空指针.                        | 现在就 drop 用户对象, 释放堆延迟到批末.       |
| `delete(self)`                | `&mut self` 还活着, 不能现在 drop. 但 fd 这一批次内不会再出现.                      | 标记 `drop_current = true`, 批末再回收.       |
| `run_once_with_timeout(...)`  | 会把当前分发状态搞乱, 还会重新进 `epoll_wait`.                                      | **panic**.                                    |

承载这一切的状态, 是一个紧凑的小结构体:

```rust,ignore
struct Handling {
    fd: RawFd,                                      // 现在是谁在跑
    drop_current: bool,                             // 自删请求
    deferred_drop: Vec<ThinBoxSubscriber<Eventp>>,  // 已 drop_in_place, 等批末 dealloc
}
```

只要我们身处一个分发批次中, `self.handling` 就是 `Some`. 在 `Some` 状态下再次进入
`run_once_with_timeout` 会直接 panic —— 这就是我们禁止重入式 `run_once` 的方式
([src/lib.rs:285-322](../../src/eventp/lib.rs.html#285-322)).

### 4.2 两种风格的 `delete`

```rust,ignore
fn delete(&mut self, fd: RawFd) -> io::Result<()> {
    // epoll_ctl(EPOLL_CTL_DEL) —— 各路径下都要做
    ...
    if let Some(h) = &mut self.handling {
        if h.fd == fd {
            // (A) 自删: registry 入口先留着, 批末才动
            h.drop_current = true;
        } else {
            // (B) 删别人: 先从 registry 摘掉, 现在就跑用户析构
            //     (好让 fd/socket 立刻释放), 但堆槽位保留到批末.
            let mut sub = self.registered.remove(&fd).unwrap();
            sub.drop_in_place();
            h.deferred_drop.push(sub);
        }
    } else {
        // (C) 不在分发循环里: 直接删
        self.registered.remove(&fd);
    }
    Ok(())
}
```

由此引出一个值得用测试钉死的用户可见的小怪癖:

- **删别人之后, 在同一个 handler 里把同一个 fd 重新 add → 成功**. (B) 已经把 registry 项摘掉,
  新 `add` 不会撞上.
- **自删之后, 在同一个 handler 里把同一个 fd 重新 add → 返回 `AlreadyExists`**. 自删只是翻了一面旗,
  registry 项还在.

这两条都有测试守着 ([handler_can_re_add_other_fd_after_delete](../../src/eventp/lib.rs.html#781),
[self_delete_then_re_add_same_fd_returns_already_exists](../../src/eventp/lib.rs.html#869)),
今后任何相关改动, 都会在测试上现形, 不会被默默改掉.

### 4.3 `ThinBoxSubscriber` 加一个哨兵字段

§2 中那个 `drop_in_place` 故事还差最后一笔. 当 (B) 提前跑完用户析构后, 堆槽位还在,
但它逻辑上"已经死了". 如果 `epoll_wait` 在同一批次里同时报告了 A 和 B, 后续分发循环还会从
`ev.data()` 重建 B 的瘦指针 —— 这时我们绝对*不能*再跑一遍用户的 `handle`.

所以 §2 中那个允诺过的 raw fd 字段终于派上用场:

```text
+---------+---------+---------+---------+--------------------+
|  _pad_  |  raw fd |  _pad_  |  vptr   | dyn Subscriber<Ep> |
+---------+---------+---------+---------+--------------------+
          ptr-16             ptr-8      ↑
                              ThinBoxSubscriber { ptr }
```

它身兼两职:

- **快路径读 fd**. 分发循环要在调 `handle()` 之前, 先把"现在是谁在跑"记到 `handling.fd` 里.
  有了缓存的 fd, 这就是一条 load —— 不必走虚函数.
- **drop-in-place 哨兵**. `drop_in_place` 在调用用户析构*之前*把 `raw_fd` 写为 -1
  (这样万一 `T::drop` 期间发生重入式访问, 看到的是"已死"状态), 而 `try_deref_mut` 一旦看到 -1
  就返回 `None` ([src/thin.rs:189-246](../../src/eventp/thin.rs.html#189-246)).

分发循环里, 每次重建出来的瘦指针都被裹在 `ManuallyDrop` 里
([src/lib.rs:333-336](../../src/eventp/lib.rs.html#333-336)). 真正的 owner 是 registry
(或 `deferred_drop`), 即便 handler 在退出时 panic, 这个本地变量也不会 double-free.

### 4.4 批末收尾

循环结束后, 我们 `take()` 走 `self.handling`, 把它清回 `None`. `Handling` 析构时会顺带 drop
`deferred_drop` 这个 vec, 进而 drop 里面每一个 `ThinBoxSubscriber`, 最后才走到 `alloc::dealloc`.
所有 (B) 中提前析构过的 subscriber, 它们的堆槽位就在这里被一并释放. 而被打了 `drop_current`
标记的 subscriber, 早在 handler 返回那一刻就已经从 registry 里被摘掉了.

---

## 5. Builder & DI: 把样板代码扔出去

每加一个 fd, 就要写一组 `struct + AsFd + HasInterest + Handler`, 还得再写一组 mock,
真的是受够了. 让我们看看类型系统能把我们带到哪里.

### 5.1 用户写出来的样子

```rust,ignore
eventp::interest()                           // 空 Interest
    .edge_triggered()                        // Interest 的 builder 方法
    .read()
    .with_fd(listener)                       // (Interest, Fd)
    .with_handler(on_connection)             // → TriSubscriber
    .register_into(&mut reactor)?;           // 调用 Eventp::add

fn on_connection(
    listener:    &mut impl Accept,
    mut reactor: Pinned<impl EventpOps>,
) { ... }
```

不需要写 subscriber struct, 不需要写 trait impl. handler 就是个普通的 `fn` (或闭包),
参数想要什么有什么, **顺序也任你排**.

### 5.2 builder 的两半: 双 trait 风格

这里没有所谓的 `Builder<T>`. `with_fd` 和 `with_handler` 是两个 trait 方法, 它们各自把一种
tuple 类型变成另一种, 而且两者顺序可换:

```rust,ignore
impl<Args, F> WithFd      for (Interest, FnHandler<Args, F>) { type Out<Fd> = TriSubscriber<Fd, Args, F>; ... }
impl<Fd: AsFd> WithHandler for (Interest, Fd)                { type Out<Args, F> = TriSubscriber<Fd, Args, F>; ... }
```

无论你先调哪一个, 终点都汇聚到 `TriSubscriber<Fd, Args, F>`. `Subscriber<Ep>` trait 对
`AsFd + HasInterest + Handler<Ep>` 有一个 blanket impl, 因此最终类型可以直接喂给 `register_into`.

### 5.3 参数注入: 一台 macro 工厂

handler 可以从 `{ &mut Fd, Event, Interest, Pinned<'_, Ep> }` 里挑出任意子集, 顺序任意.
为了在不动用 proc-macro 的前提下做到这点, 库直接用 `macro_rules!` 工厂手写出**全部 65 个 impl**
(1 个零参 + 4·P(4,1) + P(4,2) + P(4,3) + P(4,4) = 1 + 4 + 12 + 24 + 24 = 65;
见 [src/tri_subscriber.rs:143-253](../../src/eventp/tri_subscriber.rs.html#143-253)).

让这一切跑起来, 靠两个小细节:

- **用 `PhantomData<fn(Args)>` 锁住签名**. Rust 严格地说允许同一个类型 `impl FnMut<A>` 多次,
  `FnHandler<Args, F>` 自带一个 `Args` 类型参数, 因此 `(fd, event)` 和 `(event, fd)`
  对应不同的 `Args`, 两份 `Handler` impl 也就互不重叠.
- **TT-muncher 累加器** 是 `impl_handler!` 内部的写法, 它从左到右扫描参数列表, 边走边把调用
  的实参列表拼出来 —— 这是 `macro_rules!` 做 n 元代码生成的经典模式.

### 5.4 测试几乎是免费的

因为 handler 就是普通函数, reactor 操作又走 `EventpOps` trait, 你的测试可以写成:

```rust,ignore
fn on_connection<Ep: EventpOps>(listener: &mut impl Accept, mut reactor: Pinned<Ep>) { ... }

#[test]
fn accepts_then_registers_stream() {
    let mut mock_accept  = MockAccept::new();    // ← 你只需 mock 你真用到的那些
    let mut mock_reactor = MockEventp::new();

    mock_accept.expect_accept().returning(...);
    mock_reactor.expect_add().times(1).returning(|_| Ok(()));

    on_connection(&mut mock_accept, pinned!(mock_reactor));
}
```

`MockEventp` 由 [`mockall`](https://docs.rs/mockall) 生成 —— 见
[`src/mock.rs`](../../src/eventp/mock.rs.html), 而 `pinned!` 宏负责把它在栈上 pin 住,
省去 `Box::pin` 的繁文缛节 ([src/pinned.rs:82-86](../../src/eventp/pinned.rs.html#82-86)).
那些你在 `fn handle` 里没注入的参数, 一概不用 mock.

要看完整端到端的测试套件按这种风格怎么写, 请看
[`examples/echo-server.rs`](https://github.com/FuuuOverclocking/eventp/blob/main/examples/echo-server.rs).

---

## 6. 零开销的分发路径, 实测

来看一眼 `Eventp::run_once_with_timeout` 实际编译出了什么. 下面是 echo-server `--release` 构建中
内层分发循环的反汇编 (略加注释):

```text
; for ev in buf:
   17b8c: mov  rdi, [r14 + r15 + 0x4]   ; rdi  = ev.data  (subscriber 地址)
   17b91: mov  eax, [rdi - 0x10]        ; eax  = *raw_fd_ref()        ← 不走虚表
   17b94: mov  [r12], eax               ; handling.fd = eax

;     if !is_subscriber_dropped:
   17b98: cmp  eax, -1                  ; raw_fd == -1 ?
   17b9b: je   .skip                    ; 用手写的 `unlikely` 暗示分支不走
   
;         s.handle(Event::from(ev), Pinned(...))
   17b9d: mov  rax, [rdi - 0x8]         ; rax = vptr
   17ba1: mov  esi, [r14 + r15]         ; esi = ev.events  (Event::from)
   17ba5: mov  rdx, rbx                 ; rdx = &mut self  (Pinned)
   17ba8: call [rax + 0x30]             ; 一次间接调用 —— 进入 handler

;     if handling.drop_current { ... }
   17bab: cmp  byte ptr [rbx + 0x34], 0
   17baf: je   .next_event              ; 常见路径: 啥也不做
```

就这些. 每个事件的开销: 一次读 user-data, 一次读缓存 fd, 一次跳转 (基本不会走), 一次读 vtable 槽位,
一次间接调用. 没有哈希, 没有分配, 没有 `Token → Handler` 查表, 没有 trampoline.

对比一下 `event-manager` 那一套: SipHash 1-3 + 三次 `HashMap::get_mut` + 一次 `Box<dyn>` 解引用,
*每个事件*都来这么一遍. 这不是常数因子上的差距, 是一个数量级上的差距.

### 一些更安静的优化

- **`FxHashMap`**. key 是内核分配的小整数, SipHash 纯属浪费.
  ([src/lib.rs:134](../../src/eventp/lib.rs.html#134))
- **`MaybeUninit<EpollEvent>` 事件缓冲**. 分配 `capacity` 个槽, `set_len` 到 `capacity`
  但不初始化, 然后再切片到 `epoll_wait` 实际写入的前 `n` 个. `EpollEvent` 是 `libc::epoll_event`
  的 POD 包装. ([src/lib.rs:201-219](../../src/eventp/lib.rs.html#201-219))
- **`hint::unreachable_unchecked()`** 用在分发循环里, 告诉 LLVM 在某个特定点上 `self.handling`
  必然是 `None`, 省掉一次 drop check.
  ([src/lib.rs:308-322](../../src/eventp/lib.rs.html#308-322))
- **手写的 `unlikely`**, 用 `checked_div(0)` —— 一个老把戏, 不依赖 unstable intrinsic
  就能把分支提示喂给优化器.
  ([src/thin.rs:230-237](../../src/eventp/thin.rs.html#230-237))
- **`mem::transmute_copy`**, 而不是 `transmute`, 用于把瘦指针洗成 `usize` 时使用 ——
  因为我们后面还要把原值 move 进 registry.
  ([src/lib.rs:383](../../src/eventp/lib.rs.html#383))
- **`EPOLL_CTL_DEL` 直接调 `libc::epoll_ctl`**, 因为 `nix` 的封装非要一个 `AsFd` 的源头,
  而 source 也许早已被 drop. 内核其实只需要那个 fd 整数.
  ([src/lib.rs:456-463](../../src/eventp/lib.rs.html#456-463))

### 实测数据

上面那段反汇编是显微镜, 这一节是钟表.

测试代码在 [`benches/dispatch.rs`](https://github.com/FuuuOverclocking/eventp/blob/main/benches/dispatch.rs).
三个 reactor 都通过 `eventfd` 触发, 这样无论用哪个 dispatcher,
一次 fire-and-drain 都包含相同的三个 syscall (`epoll_wait`,
`eventfd_write`, `eventfd_read`): **eventp**, **mio**(配上一张 30 行的
`FxHashMap<Token, Box<dyn FnMut()>>` 用户表 —— 任何 mio 用户实际就这么写)
和 **event-manager**. 任何其他事件源都会让我们去测内核 I/O, 而不是分发本身.

**测试机:** Intel Xeon Platinum 8163 @ 2.50 GHz (Skylake-SP, 33 MB L3 共享),
Linux 5.10.134, rustc 1.95.0; `cargo bench` 配 `lto=true` 与
`codegen-units=1`(见 `Cargo.toml` 的 `[profile.bench]`). 没有 CPU 隔离 /
绑核, 看 delta, 不要看绝对值.

#### N 个已注册中只有一个 ready, 每个 subscriber 一个 fd

![每 subscriber 一个 fd 时分发单事件的延迟](https://raw.githubusercontent.com/FuuuOverclocking/eventp/refs/heads/main/docs/images/bench-dispatch-one-single-fd.svg)

| N        | eventp     | event-manager | mio + FxHashMap | em − ep |
|----------|------------|---------------|-----------------|---------|
| 1        | 1.126 µs   | 1.165 µs      | 1.133 µs        | +39 ns  |
| 10       | 1.112      | 1.163         | 1.136           | +51 ns  |
| 100      | 1.114      | 1.165         | 1.138           | +51 ns  |
| 1 000    | 1.108      | 1.159         | 1.130           | +51 ns  |
| 10 000   | 1.103      | 1.157         | 1.127           | +54 ns  |
| 100 000  | 1.127      | 1.179         | 1.153           | +52 ns  |

这张表能读出三件事:

1. **三家的分发都是真 O(1)**. N=1 到 N=10 000 之间, 每一行的中位数变化都不到 25 ns.
   没有一家的"找回 handler"开销随注册数增长.
2. **N=100 000 那个台阶是大家共同的**. 每家都慢了 ~25 ns. 如果这是 HashMap 的 cache 压力,
   那只有 event-manager 应该感受到; 三家齐步上涨说明这笔账记在内核侧 ——
   epoll interest set 的内部数据结构开始感受 100k 条目了, 跟用户态无关.
3. **稳定的 ~50 ns 差距是两次 SipHash 查询**.
   event-manager 的热路径先做 `fd_dispatch.get(fd)`, 再做
   `subscribers.get_mut_unchecked(id)`, 两个都是
   `std::collections::HashMap` (SipHash 1-3). mio 在 eventp 上面 ~25 ns,
   一次 FxHash 查询. FxHash 比 SipHash 大致快 2 倍, 数字对得上.

#### 第三次 HashMap 在哪里真的发生

`dispatch_one_multi_fd_M4` 这一组每个逻辑 subscriber 注册 4 个 eventfd ——
这正是一个 virtio device, 一个 vsock backend, 或者任何"一个组件管几个信号 fd"
都会自然写成的形状.

![每 subscriber 四个 fd 时分发单事件的延迟](https://raw.githubusercontent.com/FuuuOverclocking/eventp/refs/heads/main/docs/images/bench-dispatch-one-multi-fd.svg)

| N (sub 数) | eventp     | event-manager | mio        | em − ep |
|-----------|------------|---------------|------------|---------|
| 100       | 1.109 µs   | 1.212 µs      | 1.161 µs   | +103 ns |
| 1 000     | 1.125      | 1.207         | 1.147      | +82 ns  |
| 10 000    | 1.125      | 1.209         | 1.159      | +84 ns  |

eventp 和 mio 相比 single-fd 几乎没变. event-manager 在原本的 50 ns 之上
**又多了 ~30 ns** —— 正是 §1.1 预言的那次第三 HashMap. 一个 sub 持有 4 个 fd 时,
`process(events: Events, ...)` 拿到的只有 `RawFd`, 想对正确的那个 owned
`EventFd` 调 `read`, handler 必须自己写
`self.fds.get_mut(&events.fd())`. 在 event-manager 的 API 形状里,
要躲开这次查询只能滑向 `unsafe` + 裸 `RawFd` 存储, 没有干净的出路.
eventp 不付这笔费用, 因为 fd 对象就作为字段挂在 subscriber 上,
通过 §5 那一套依赖注入直接以 `&mut Fd` 喂到 handler 里.

#### 摊销吞吐量

`dispatch_all_ready`: N 个 subscriber 同时 fire, 一次 `run_once` 把这一批全部分发完.

![per-event 摊销吞吐](https://raw.githubusercontent.com/FuuuOverclocking/eventp/refs/heads/main/docs/images/bench-dispatch-all-ready.svg)

| N      | eventp ns/event | event-manager ns/event | mio ns/event |
|--------|-----------------|------------------------|--------------|
| 16     | 804             | 856                    | 828          |
| 64     | 809             | 862                    | 833          |
| 256    | 806             | 866                    | 837          |
| 1 024  | 817             | 896                    | 855          |

单核吞吐: eventp ≈ **1.24 M 事件/s**, event-manager ≈ 1.16 M,
mio + FxHashMap ≈ 1.20 M.

em−ep 的差从 N=16 时的 +52 ns 拉到 N=1024 时的 +79 ns —— 多出来这 +27 ns
就是 event-manager 的 HashMap 条目从 L1 data cache 溢出去了
(1024 条 × ~24 字节 ≈ 24 KB, 刚好越过本机 32 KB 的 L1d). eventp 根本没有
hashtable 可以 miss.

#### 关于绝对值的一句话

内核那三个 syscall 在 1.1 µs 总时间里大概占 1.05 µs —— 当下一个事件的 ~95%.
也就是说, 把 event-manager 换成 eventp, 在这个合成 eventfd benchmark 上
省的是单事件的 4–7%. 单看这个数字确实小.

但有意思的轴是未来, 不是当下. 当 syscall 这层消失下去
(io_uring 配 `IORING_SETUP_IOPOLL`, 批量轮询 ring, 在 NAPI 设备上 busy-poll,
甚至 kernel bypass), 这一节量到的分发开销才是真正剩下的部分.
那时候, 同样这 50 ns 就是大头, 不再是舍入误差. eventp 的形状是为那种未来准备的,
不是为当下"syscall 主导一切"的局面准备的.

---

## 7. 已知限制

- **`Eventp` 不是 `Send` 的**. 跨线程访问需要走
  [`remote_endpoint`](mod@crate::remote_endpoint) 模块, 它通过 `eventfd` + MPSC channel
  把闭包送进 reactor. 让 `Eventp` 自身变成 `Send`, 意味着重新审视 §3-§4 中的若干 unsafe 不变式,
  目前没有这个计划.
- **仅支持 64 位 Linux**. 两者都在编译期校验
  ([src/lib.rs:1-11](../../src/eventp/lib.rs.html#1-11),
  [src/thin.rs:48-49](../../src/eventp/thin.rs.html#48-49));
  移植到 32 位意味着放弃"把地址塞进 `u64`"这一招, 而那是这个库存在的全部意义.
