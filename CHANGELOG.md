## 1.0.0 (2026-10-22)


### Features

* add bake command assembling the export ([dbf2287](https://github.com/greenroom-robotics/pixi-chef/commit/dbf2287602a59ff76008f409027a06a44c873702))
* add cook command skipping source packages ([363f48c](https://github.com/greenroom-robotics/pixi-chef/commit/363f48cd5c5df8bc932bc2e0a512d620e3f31c1c))
* add pixi-chef prepare, cook and bake ([634c1a3](https://github.com/greenroom-robotics/pixi-chef/commit/634c1a381a8c9dd7f463983ffe244dd1553cacf9))
* add prepare --into and --include options ([74fea0a](https://github.com/greenroom-robotics/pixi-chef/commit/74fea0a960a308d59b07e6811957a501e05e36d4))
* add prepare command writing the recipe ([b8bd5f8](https://github.com/greenroom-robotics/pixi-chef/commit/b8bd5f83a6a44b47f75ee8fcb8948d6d8ab8beda))
* add release workflow for conda package and scratch image ([54abd59](https://github.com/greenroom-robotics/pixi-chef/commit/54abd596b6eb1f6524c1b36c089054ab7e71f735))
* reject --into when manifest paths shift and fail bake on missing activation scripts ([d0b58f6](https://github.com/greenroom-robotics/pixi-chef/commit/d0b58f60f7d35351d9bfce2c06942be8b90296b1))


### Bug Fixes

* address cook review findings ([3f72301](https://github.com/greenroom-robotics/pixi-chef/commit/3f72301bf6cc9a34cfdc5619d639ea1ed5123afa))
* avoid as-conversion in platform filter test ([b363e20](https://github.com/greenroom-robotics/pixi-chef/commit/b363e20dd69715ba535097deb7af041263923aa2))
* box lock parse error to satisfy clippy ([b421f0d](https://github.com/greenroom-robotics/pixi-chef/commit/b421f0da1fca68095270b9c5032fca32d6d4b844))
* build image from release tags in a separate workflow ([1fc8f02](https://github.com/greenroom-robotics/pixi-chef/commit/1fc8f02923abcbd7cb732e15c0637cac3254eede))
* capture host-only source packages and cover failure paths ([abe4d47](https://github.com/greenroom-robotics/pixi-chef/commit/abe4d47b92d06330c3f518837e23585fa9ebe84d))
* check every lock path resolves alike under --into ([39d9405](https://github.com/greenroom-robotics/pixi-chef/commit/39d9405e04a5e7fa86a76f7a6392db774792c6f9))
* gate image on release, pin sha, smoke test before tagging ([86c1501](https://github.com/greenroom-robotics/pixi-chef/commit/86c1501d23ec9900080fd9bbdf3448b71fd6895a))
* harden bake staging, hook PATH and sourced files ([b25836a](https://github.com/greenroom-robotics/pixi-chef/commit/b25836a27383e89bafcbade5581b0100dcf04937))
* keep pixi hook export order and tighten e2e PATH checks ([1df320b](https://github.com/greenroom-robotics/pixi-chef/commit/1df320b2356316f8a76169298abe4be66c258162))
* make bake export self-contained with env bash and real-pixi test ([4743031](https://github.com/greenroom-robotics/pixi-chef/commit/47430313e483f0d5ffab348ba286f931cdb7b6c6))
* pin pixi 0.82.0, cover platform filter and pypi wheels ([e3c6059](https://github.com/greenroom-robotics/pixi-chef/commit/e3c6059356a2c8fc2979a1eebc58ff3b881459be))
* restrict --into to sibling dirs and isolate include walk ([4c65bc1](https://github.com/greenroom-robotics/pixi-chef/commit/4c65bc1eb985fdc4e22cccedee3e64103707cd64))
* reword --into help to match per-path check ([366f4d9](https://github.com/greenroom-robotics/pixi-chef/commit/366f4d9b0ede9397032e2df76ad743c6993fe22b))
* trigger image build on published release instead of tag push ([f238954](https://github.com/greenroom-robotics/pixi-chef/commit/f238954bd97b3a38fb3f26bd0d38370a343e0786))
* validate prepare --into and --include inputs ([70e2176](https://github.com/greenroom-robotics/pixi-chef/commit/70e2176d1614d5f06a70cd0fdb8c14be5393cc38))
