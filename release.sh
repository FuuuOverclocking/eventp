#!/bin/bash

set -e

current_version=$(awk -F '"' '/^version =/ {print $2}' "Cargo.toml")
new_version="$1"

sed -i "s/^version = \"$current_version\"/version = \"$new_version\"/" Cargo.toml
sed -i "s/$current_version/$new_version/g" README.md

cargo clippy --all-features
git cliff --tag $new_version > CHANGELOG.md

git add .

git commit -s -m "v${new_version}"
git tag "v${new_version}" -m "v${new_version}"

git push --follow-tags
cargo publish --registry crates-io
