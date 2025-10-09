# Changelog

All notable changes to this project will be documented in this file. See [conventional commits](https://www.conventionalcommits.org/) for commit guidelines.

---
## [1.0.0-rc.6](https://github.com/FuuuOverclocking/eventp/compare/v1.0.0-rc.5..1.0.0-rc.6) - 2025-10-09

### Documentation

- Add images and text for technical.zh.md - ([3826c8e](https://github.com/FuuuOverclocking/eventp/commit/3826c8e2652aa899a6d5e3240558e637a6a7391a)) - Fuu

### Features

- Add remote-endpoint feature - ([2b0f4db](https://github.com/FuuuOverclocking/eventp/commit/2b0f4db6846691924025686accd71de5ba96f303)) - Fuu

### Refactoring

- Avoid an unnecessary virtual function call - ([3d6ec14](https://github.com/FuuuOverclocking/eventp/commit/3d6ec140a442d760d3d240e5d1ebdb51ee71cc12)) - Fuu
- Add raw fd as first field to layout of ThinBoxSubscriber.. - ([8df722f](https://github.com/FuuuOverclocking/eventp/commit/8df722ffdd7c2c78389e3f386df527f451c83810)) - Fuu

---
## [1.0.0-rc.5](https://github.com/FuuuOverclocking/eventp/compare/v1.0.0-rc.4..v1.0.0-rc.5) - 2025-10-09

### Documentation

- Add docs and images for crate - ([d5a5c5e](https://github.com/FuuuOverclocking/eventp/commit/d5a5c5e27f113e9bd75aefb8c301f2cfb5739f37)) - Fuu

### Refactoring

-  [**breaking**]Role of `Registry` replaced by `EventpOpsAdd` - ([3205579](https://github.com/FuuuOverclocking/eventp/commit/32055792b7d64e8e84df76f21de2aafd02a1bd24)) - Fuu

---
## [1.0.0-rc.4](https://github.com/FuuuOverclocking/eventp/compare/v1.0.0-rc.3..v1.0.0-rc.4) - 2025-10-06

### Documentation

- Add or adjust some documentation - ([6b2f2f0](https://github.com/FuuuOverclocking/eventp/commit/6b2f2f02a8a7bd87333518fe9525e5a48f5d72e1)) - Fuu

### Refactoring

-  [**breaking**]Remove bin_subscriber - ([53fc2fa](https://github.com/FuuuOverclocking/eventp/commit/53fc2fa5d733080ff85ce1532820410cc5ca69d1)) - Fuu
-  [**breaking**]impl From<Box<S>> for ThinBoxSubscriber instead of From<S> - ([6751ff7](https://github.com/FuuuOverclocking/eventp/commit/6751ff76395f2d3dda0a8378a5eb0aab34578fc1)) - Fuu

### Ci

- Update GitHub workflows - ([f5f0a20](https://github.com/FuuuOverclocking/eventp/commit/f5f0a2057ff618fb9beb3488525362e5fe542d76)) - Fuu

---
## [1.0.0-rc.3](https://github.com/FuuuOverclocking/eventp/compare/v1.0.0-rc.2..v1.0.0-rc.3) - 2025-10-02

### Documentation

- Add documentation for `Event` and `Interest` - ([e447f64](https://github.com/FuuuOverclocking/eventp/commit/e447f64a1e554cf93ffaec5865c9618806168b09)) - Fuu
- Add documentation for several items - ([bebf9ac](https://github.com/FuuuOverclocking/eventp/commit/bebf9accb38af0b88c3ebcbbb8f8c94fc928faed)) - Fuu

### Miscellaneous Chores

- Add .vscode to .gitignore - ([194216b](https://github.com/FuuuOverclocking/eventp/commit/194216b142179e7ff13482c0867e71a508390297)) - Fuu
- Replace release.sh with Makefile - ([383a97a](https://github.com/FuuuOverclocking/eventp/commit/383a97a5204bc3c542ef1904e5e9ca1a2a0054f0)) - Fuu

---
## [1.0.0-rc.2](https://github.com/FuuuOverclocking/eventp/compare/v1.0.0-rc.1..v1.0.0-rc.2) - 2025-09-28

### Documentation

- Fix broken docsrs builds - ([3564635](https://github.com/FuuuOverclocking/eventp/commit/3564635f56dec79bf3af910ac8282efa92dbdc45)) - Fuu

---
## [1.0.0-rc.1](https://github.com/FuuuOverclocking/eventp/compare/v0.3.3..v1.0.0-rc.1) - 2025-09-28

### Documentation

- Update README - ([0653a4d](https://github.com/FuuuOverclocking/eventp/commit/0653a4d14b8a4cd9380349c8632c08812f282b89)) - Fuu
- Use English comments - ([c9d1f8d](https://github.com/FuuuOverclocking/eventp/commit/c9d1f8da191b3f3c515ab6d46f5e16ffebec3c23)) - Fuu

### Refactoring

-  [**breaking**]Remove `interest` from `Handler`'s parameter - ([a498244](https://github.com/FuuuOverclocking/eventp/commit/a498244f4ff0cd2d3a5d81d02cf0cb790b7d8e5e)) - Fuu
- Move `MockEventp` to standalone module.. - ([9140cdd](https://github.com/FuuuOverclocking/eventp/commit/9140cdd41bb6373364bb3e8c1817653f7da220b5)) - Fuu
-  [**breaking**]Rename `WithInterest` to `HasInterest`.. - ([8d8ae40](https://github.com/FuuuOverclocking/eventp/commit/8d8ae4097f149b4542c765199622a6aaea7497d0)) - Fuu
-  [**breaking**]Adjust module exports and remove `FdWithInterest` - ([3a32c27](https://github.com/FuuuOverclocking/eventp/commit/3a32c27a41778d957da5bb78c9ed4a336c8824cf)) - Fuu

### Build

- Correct settings in cliff.toml - ([4ce84b2](https://github.com/FuuuOverclocking/eventp/commit/4ce84b2695afbfb7012246a5e99d504472db6fb4)) - Fuu

---
## [0.3.3](https://github.com/FuuuOverclocking/eventp/compare/v0.3.2..v0.3.3) - 2025-09-27

### Documentation

- Fix builds on docsrs - ([466b9b0](https://github.com/FuuuOverclocking/eventp/commit/466b9b0b6b32fc0d1eec50c279fcc08c9b92325c)) - Fuu

### Build

- Add release.sh - ([fd639c7](https://github.com/FuuuOverclocking/eventp/commit/fd639c7f1db3efa6a6035abe774aa7e7309c0b4c)) - Fuu

---
## [0.3.2](https://github.com/FuuuOverclocking/eventp/compare/v0.3.1..v0.3.2) - 2025-09-27

### Documentation

- **(example)** Add comments for echo-server - ([587d521](https://github.com/FuuuOverclocking/eventp/commit/587d52141fff5824105b10d73cfb307a9940977b)) - Fuu
- let docsrs know mock feature and update README - ([f998d76](https://github.com/FuuuOverclocking/eventp/commit/f998d76bd6c47e8fae7a5145185c3537ccef883e)) - Fuu

---
## [0.3.1](https://github.com/FuuuOverclocking/eventp/compare/v0.3.0..v0.3.1) - 2025-09-26

### Tests

- Remove broken doctest of Interest - ([bba2c03](https://github.com/FuuuOverclocking/eventp/commit/bba2c03bbdbf846943a0e71bcfe5d5375a62a5e7)) - Fuu

---
## [0.3.0](https://github.com/FuuuOverclocking/eventp/compare/v0.2.0..v0.3.0) - 2025-09-26

### Features

-  [**breaking**]Remove unused query methods from Interest - ([4d8f68f](https://github.com/FuuuOverclocking/eventp/commit/4d8f68ffacfdc3d504769c03ab8a973bf7d50e03)) - Fuu
- Add remove_xxx methods for Interest flags - ([4d8501a](https://github.com/FuuuOverclocking/eventp/commit/4d8501a6baf8c7fed366833df2322b857480796a)) - Fuu

### Miscellaneous Chores

- Add license and update documentation - ([b24ab33](https://github.com/FuuuOverclocking/eventp/commit/b24ab3300cfc70af88c1e48ca434e221d08d16e0)) - Fuu
- Update README - ([5a0d840](https://github.com/FuuuOverclocking/eventp/commit/5a0d840d6885be21ba6dd95792d22303973f81c3)) - Fuu
- Use cocogitto-style changelog - ([fa023dc](https://github.com/FuuuOverclocking/eventp/commit/fa023dcdce9dfb4745426ba49c2c1bdf1bb9723f)) - Fuu

### Style

- Rename generic parameter E: EventOps to Ep - ([2bca3db](https://github.com/FuuuOverclocking/eventp/commit/2bca3dbd1efe22d64701cba6d221ea3457ca6e59)) - Fuu

### Build

- Remove the unused dependency - ([d7fb26b](https://github.com/FuuuOverclocking/eventp/commit/d7fb26bb6cf3c004415d63ec2a0208fa0557302d)) - Fuu

### Ci

- Setup GitHub workflows - ([c805fa4](https://github.com/FuuuOverclocking/eventp/commit/c805fa498d037becdb07035d38496c44d1ba5d1f)) - Fuu

---
## [0.2.0] - 2025-09-25

### Features

- **(examples)** Demonstrate DI-style callbacks - ([0a2fb31](https://github.com/FuuuOverclocking/eventp/commit/0a2fb3100536459aab09f178a3b67f56448f44aa)) - Fuu
- Add PhantomPinned to Eventp struct - ([115de9d](https://github.com/FuuuOverclocking/eventp/commit/115de9d37de44775fb2b12fdacbbb45b3800f156)) - Fuu

### Miscellaneous Chores

- use git-cliff to generate changelog - ([974db7b](https://github.com/FuuuOverclocking/eventp/commit/974db7be0a78054182f4fe244b1af04f6327eee8)) - Fuu
- add cargo manifest - ([b280113](https://github.com/FuuuOverclocking/eventp/commit/b2801136161f14520b3eccbbeb649726e6bc7c0f)) - Fuu
- Edit README - ([2319fd1](https://github.com/FuuuOverclocking/eventp/commit/2319fd19092b023cc2bf35c96e560bfda7cbb26a)) - Fuu

### Refactoring

-  [**breaking**]Replace `Pin<&mut Ep>` with wrapper - ([3837664](https://github.com/FuuuOverclocking/eventp/commit/3837664bb54572c1ce50ba02701c59ce703a538b)) - Fuu

<!-- generated by git-cliff -->
