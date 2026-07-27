build:
	mkdir -p target
	jam-pvm-build --module service --output target/parachain-service.jam service
