.PHONY: check doc fmt release

CURRENT_VERSION := $(shell awk -F '"' '/^version =/ {print $$2; exit}' Cargo.toml)

check:
	cargo clippy --example=echo-server
	cargo clippy --example=echo-server --all-features
	cargo test --all-features --example=echo-server
	cargo +nightly fmt --check
	cargo test --all-features

doc:
	RUSTDOCFLAGS="--cfg docsrs" cargo +nightly doc --all-features

fmt:
	cargo +nightly fmt

release:
ifndef VERSION
	$(error Please specify VERSION, e.g., make release VERSION=1.2.3)
endif
	@echo "Current version: $(CURRENT_VERSION)"
	@echo "New version: $(VERSION)"
	@echo "Updating version in Cargo.toml and README.md..."

	sed -i "s/^version = \"$(CURRENT_VERSION)\"/version = \"$(VERSION)\"/" Cargo.toml
	sed -i "s/$(CURRENT_VERSION)/$(VERSION)/g" README.md

	cargo clippy --all-features
	git cliff --tag $(VERSION) > CHANGELOG.md

	git add .
	git commit -s -m "v$(VERSION)"
	git tag "v$(VERSION)" -m "v$(VERSION)"

	git push --follow-tags
	cargo publish --registry crates-io

	@echo "Release v$(VERSION) completed successfully!"
