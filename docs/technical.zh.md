# Technical (中文)

[English](crate::_technical) | 中文

`Eventp` 具有零开销的事件分发机制和简洁、测试友好的 API.

## 与 mio, EventManager 的对比

相较于 [mio](https://docs.rs/mio/latest/mio/) (以及更裸的 nix, libc), [EventManager](https://docs.rs/event-manager/latest/event_manager/)
添加了一层订阅和事件分发机制. 它能够将事件处理由静态的 `match` 代码结构, 分解为运行时灵活动态的注册修改,
这很好, 对于大型工程项目非常有帮助.

例子:
- mio: [examples/tcp_server.rs](https://github.com/tokio-rs/mio/blob/master/examples/tcp_server.rs)
- event-manager: [Basic Single Thread Subscriber](https://github.com/rust-vmm/event-manager?tab=readme-ov-file#basic-single-thread-subscriber)

然而 event-manager 发现管理好所有权很困难. 因为一个事件触发后, 订阅者往往会想要向 `epoll` 中添加或修改事件源
(比如当它是一个 TCP Listener 时, 会想要添加新的 TCP 连接); 同时, 它也想要获取对自身数据的可变引用. 这就带来了双重可变引用的问题.

```rust,ignore
fn handler(myself: &mut Subscriber, reactor: &mut Reactor) {
                   ^^^^^^^^^^^^^^^           ^^^^^^^^^^^^
                   error: 双重可变引用, Subscriber 是 Reactor 的一部分, 来自同一个对象
}
```

为了绕过这个问题, 它用了足足 4 个 `HashMap`, 创建了 `EventManager - Subscriber - fds` 三层结构:

![event-manager](https://raw.githubusercontent.com/FuuuOverclocking/eventp/refs/heads/main/docs/images/event-manager.svg)

这样, 两个可变引用就可以分别取自 `EventManager` (中的 epoll 部分) 和 `Subscriber` 了, 解决了问题.
然而代价却是对于每个触发的事件, 都要查询 3 次 `HashMap`.

更糟的是, 它的数据结构选取了 `std::collections::HashMap`. 它默认使用 SipHash 1-3 作为哈希算法,
能够抵抗 HashDos 攻击, 然而我们的 key 是 OS 给出的 fd 小整数, 这就不必要地拖慢了速度.

### 致命隐患: fd 复用导致的事件错投

将 `RawFd` 当作 subscriber 的稳定主键, 还埋下了一个更隐蔽、更危险的问题. POSIX 明确规定,
[`open(2)`](https://man.archlinux.org/man/open.2.en) 等创建 fd 的系统调用必须返回**当前进程中数值最小的未被使用的 fd**.
这意味着, 一旦某个 fd 被 `close(2)` 释放, 它的整数值会立刻成为下一次 `open`/`accept`/`socket`/`pipe` 的首选,
新打开的资源会**复用同一个 `RawFd` 编号**.

考虑这样一个并不少见的时序:

1. subscriber A 持有 `fd = 7`, 已注册到 `EventManager`, 并在它内部的 4 个 HashMap 中各占一席.
2. A 自己 close 了 `fd = 7`, 但**忘记** (或来不及) 调用 `EventManager::remove_subscriber`.
3. 进程稍后 `accept` 出一个新的连接, 内核回收 `7` 作为它的 fd, 应用又把它注册成 subscriber B.
4. epoll 分发事件时, `EventManager` 用 `fd = 7` 查表, 命中 A 的 subscriber id —— 事件被错投到了已经"死了"的 A 身上.

这是一类典型的"幽灵事件". 它的危害有三:

- **静默的逻辑错误**: 编译器无法察觉, 单元测试也极难复现, 通常只在生产环境的高并发场景下偶发翻车.
- **跨越所有权边界**: 即使 A 的对象本身已经被释放, 只要 `EventManager` 中还残留它的 `RawFd → subscriber id`
  映射, 就会有事件被路由到错误的处理者上, 甚至可能命中一段被复用的内存.
- **并非"用户的锅"**: 库的 API 形状鼓励了"先 close fd 再 remove"的写法 (尤其是当 close 由更深层的析构链触发时),
  把这种细节甩给用户去保证, 是设计的失败.

`eventp` 由于直接将 subscriber 对象的地址塞进 `epoll_event.data`, 路由不依赖 `RawFd` —— 内核回报的就是当年注册时给出的那个对象,
天然不会发生"事件错投"; 而 `Eventp::delete` 又强制把 epoll 注销和 subscriber 对象的释放绑定在一起,
连"忘记 remove"的可能性都从 API 上消除了.

### Insight

在向 `epoll` 注册 fd 时, Linux 允许一同添加一个自定义的 `u64`. 如果我们将其视作事件上下文对象的地址,
就可以 `callq` 单条指令直达对象的方法, 免去中间一切额外开销 (当然, 运行时多态还是免不了虚表的开销).

## 胖指针问题

实践中, 类似 vmm 这样的场景里大约 90% 的情况使用运行时多态 (因为泛型参数会一路向上传染到最顶层, 很麻烦).
这提出了一个 Rust 特有的胖指针问题:

<figure style="display: inline-block;">
<img src="https://raw.githubusercontent.com/FuuuOverclocking/eventp/refs/heads/main/docs/images/pointer-meta.svg" alt="Rust 胖指针" />
<figcaption style="text-align: center;">Rust 胖指针</figcaption>
</figure>

<figure style="display: inline-block;">
<img src="https://raw.githubusercontent.com/FuuuOverclocking/eventp/refs/heads/main/docs/images/cpp-vptr.svg" alt="C++ 单继承对象指针" />
<figcaption style="text-align: center;">C++ 单继承对象指针</figcaption>
</figure>

x86-64 下, 一个 `&dyn Subscriber` 的 size 为 16 个字节, 是没办法放进 `epoll` data 里的.

### Insight

不必局限于 rustc 的内存布局, 若有需要便可亲自操刀分配回收. 模仿 C++ 单继承对象布局, 自行计算 `Layout`, 把 vptr “放进” 对象内部.

### How to?

我们创建了一个类型 [`ThinBoxSubscriber`](crate::thin::ThinBoxSubscriber) 来做到这点. 让我们一步步完成这个 thin pointer
及其堆上数据的构建.

```rust,ignore
// thin.rs

pub struct ThinBoxSubscriber {
    ptr: NonNull<u8>,
    _marker: PhantomData<dyn Subscriber>,
}

impl ThinBoxSubscriber {
    pub fn new<S: Subscriber>(value: S) -> Self {
        todo!()
    }
}
```

#### Step 1: 排除例外因素

Rust 另一个特色是拥有 Zero-sized Type (ZST), 它的 size 为 0. 比如 `()`, `PhantomData<T>`, `struct Foo;` 等.
我们先把它们直接排除掉. 另外, 我们也不想支持非 64 位平台.

这两个问题都是可以解决的, 但价值不大, 我们先不花这个力气.

```rust,ignore
if size_of::<S>() == 0 {
    panic!("ZST not supported");
}

#[cfg(not(target_pointer_width = "64"))]
compile_error!("Platforms with pointer width other than 64 are not supported.");
```

于是, 我们可以断言, 胖指针的大小一定是 16 个字节:

```rust,ignore
const DYN_SUBSCRIBER_SIZE: usize = size_of::<&dyn Subscriber>();
const _: () = assert!(DYN_SUBSCRIBER_SIZE == 16);
```

#### Step 2: 施展黑魔法, reinterprets 胖指针, 取出 vptr

```rust,ignore
let fat_ptr = &value as &dyn Subscriber;
let vtable_ptr = unsafe {
    mem::transmute::<_, (usize, usize)>(fat_ptr).1
};
```

接下来, 我们就可以创建一个新的结构体布局了, 它会将 vptr 作为首个字段:

![step-2](https://raw.githubusercontent.com/FuuuOverclocking/eventp/refs/heads/main/docs/images/step-2.svg)

**微小而致命: align 空隙问题**

这里潜藏着一个致命问题, 当类型 `S` 的 align 大于一个 `usize` 时, `vptr` 和 `S` 之间会有空隙,
我们在 Deref 时不能保证 S 在那里!

![step-2-align-issue](https://raw.githubusercontent.com/FuuuOverclocking/eventp/refs/heads/main/docs/images/step-2-align-issue.svg)

**黑魔法进阶: repr(C) 的 align 一定是所有成员的最大值, 且是 2 的幂次**

因此, 我们稍稍调整了 vptr 的位置. 它仍是第一个字段, 但紧贴着 S, 把上方留给空隙. 接着, 我们让 ptr 指向 `S` 的起始位置.

> 思考: 为什么 vptr 的位置合法?

![step-2-align-issue-solved](https://raw.githubusercontent.com/FuuuOverclocking/eventp/refs/heads/main/docs/images/step-2-align-issue-solved.svg)

#### Step 3: 分配和拷贝

我们已经算好了 layout, 现在只需分配空间和拷贝数据, 便得到了 `ThinBoxSubscriber`:

```rust,ignore
let ptr = {
    let ptr = alloc::alloc(layout);
    if ptr.is_null() {
        alloc::handle_alloc_error(layout);
    }

    let ptr = ptr.add(offset_of_S);
    NonNull::new_unchecked(ptr)
};

ptr.as_ptr().sub(size_of::<usize>()).cast::<usize>().write(vtable_ptr);
ptr.as_ptr().cast::<S>().write(value);
```

#### 处理 Deref 和 Drop

Deref 的过程是轻松愉快的, `&dyn Subscriber` 是怎么没的, 我们再怎么把它变回来:

```rust,ignore
let value = self.ptr.as_ptr();
let metadata = self.ptr.as_ptr().sub(size_of::<usize>()).cast::<usize>().read();
let fat_ptr = mem::transmute::<_, *mut dyn Subscriber>((value, metadata));

&mut *fat_ptr
```

而 Drop 就稍稍有些技巧了, 我们要首先执行 `S` 的 drop, 然后 dealloc 堆空间.

由于 [RFC panic-in-drop](https://github.com/Amanieu/rfcs/blob/panic-in-drop/text/0000-panic-in-drop.md)
(提案已取消) 等相关讨论仍在继续, 当前对于发生在 `Drop` impl 中的 panic, Rust 的行为仍然是 unwind. 因此这里存在一种在
`fn drop` 的内部创建一个 `DropGuard` 的技巧. 它可以保证即使 drop `S` 的过程中发生 panic, 堆空间仍然能被回收. 详情请查看源码.

#### 与实际代码的出入

实际要稍稍复杂一些:
- 为减少热点路径的虚函数调用, 在 vptr 之外, 我们还向堆空间放入了一个 raw fd
- `Subscriber<Ep>` 实际上具有泛型参数

## 所有权问题

回到开头提出的双重可变引用问题. 假设我们只有一个 HashMap, 并且用户希望我们提供一个这样的接口:

```rust,ignore
trait Subscriber {
    fn handle(&mut self, reactor: &mut Eventp);
}
```

但是这显然违背了 Rust 的规则, 它会忍不住抱怨 `cannot borrow Eventp as mutable more than once at a time`,
阻止你通过编译.

### Descending deeper into the dark arts

在我们彻底陷入黑魔法前, 首先把 [The Problem With Single-threaded Shared Mutability](https://manishearth.github.io/blog/2015/05/17/the-problem-with-shared-mutability/)
牢记在心, 它是我们回来的路.

```rust,ignore
use rustc_hash::FxHashMap; // 采用高性能 Hash 算法, 不抗 DDOS

struct Eventp {
    registered: FxHashMap<RawFd, ThinBoxSubscriber>,
    // ...
}
```

我们知道可以从 [`RefCell`](std::cell::RefCell) 获取单线程共享可变性, 这样即便从
`&*Eventp.registered[raw_fd]` (即 `&dyn Subscriber`) 也可以取出 `&mut dyn Subscriber`. 如此, 问题分解成两步,

1. 我们能否证明, 假设 `Subscriber` 包裹在 `RefCell` 中, 它的 ref count 是不是不可能大于 1?
    - 如果可以证明清楚, 也就不用特地包装一下了, 毕竟引用计数还是有一点点开销的
2. 假设我们取走了 `&*Eventp.registered[raw_fd]`, 在 `Subscriber::handle` 调用期间 `&mut Eventp`
   所给予的数据修改能力, 是否会导致引用失效?

问题 1 取决于具体的代码实现, 如果代码中的字段引用较少, 便很容易把它说清楚. 因此, 让我们先来探究看起来更困难的问题 2.

根据定义, `&mut T` 提供了 3 种修改能力:

1. 通过 `pub field`, 即 T 的公开字段 —— 这点很容易保证, 只需使它没有公开字段
2. 通过 `pub fn method(&mut self)`, 即 `T` 的公开方法 —— 这点虽然麻烦, 至少也掌握在我们自己手里
3. `mem::take`, `mem::replace`, `*t = new_t` —— 我们完蛋了 💥

一旦用户在 `handle` 过程中做出这种匪夷所思的举动, 旧的 `Eventp` 便可能当场析构, 连带着全体 `Subscriber` 一起去世.
`Subscriber::handle` 的第一个参数 `&mut self` 也就失去了合法存在的理由!

### 🚑 是否还有抢救的余地?

有的. 回忆一下, 上一位差点去世叫做 [`Future`](core::future::Future), 和它的境况十分类似. 那时, 人们为了挽救它,
召唤出了 Rust 中最可怕的黑魔王 —— [`Pin`](core::pin).

抛开 [`Pin`](core::pin) 长达 16 章节的惊人文档不提, 它的作用非常简单, 那就是阻止 safe Rust 代码从 `Pin<&mut T>`
获得 `&mut T`, 除非 `T` 是 [`Unpin`](core::marker::Unpin) 的. 相对的, 可以在 `Pin<&mut T>` 上调用一类奇特的方法,
它们具有签名 `fn method(self: Pin<&mut Self>)`. 这些方法遵守了某种承诺, 节制地利用 unsafe 取出 `&mut Self`,
实现自己的功能, 同时保证不会把数据从中移出.

这恰好成了我们的救命稻草, 阻止了那些用户的离谱举动.

```rust,ignore
trait Subscriber {
    fn handle(&mut self, reactor: Pin<&mut Eventp>);
                                           ^^^^^^ 令它是 !Unpin 的
}
```

### `&mut Eventp` 和 `Pin<&mut Eventp>` 上的方法

与 [epoll_ctl(2)](https://man.archlinux.org/man/epoll_ctl.2.en) 对应, 这两个类型都提供了

- `.add(subscriber)`: 添加订阅者
- `.modify(raw_fd, interest)`: 修改 interest
- `.delete(raw_fd)`: 删除订阅者

此外, `&mut Eventp` 还单独提供了方法 `.run_once_with_timeout()` 来运行一次
[epoll_wait(2)](https://man.archlinux.org/man/epoll_wait.2.en) 以及分发事件.

如前面所说的, 我们实现时要小心两件事:

1. 不要造成正在处理事件的 subscriber 引用失效 (`&*Eventp.registered[raw_fd]`)
2. 不要“借出”两个或更多的可变引用

## 仍存在的缺陷

- 现阶段无法使 Eventp 成为 Send 的
