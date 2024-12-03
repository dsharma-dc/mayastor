use io_engine::core::{CoreError, UntypedBdevHandle};

pub async fn write_some(
    nexus_name: &str,
    offset: u64,
    num_blocks: u32,
    fill: u8,
) -> Result<(), CoreError> {
    write_blocks(nexus_name, offset, num_blocks, fill).await?;
    Ok(())
}

pub async fn write_blocks(
    nexus_name: &str,
    offset: u64,
    count: u32,
    fill: u8,
) -> Result<u64, CoreError> {
    let h = UntypedBdevHandle::open(nexus_name, true, false)?;
    let buflen = u64::from(h.get_bdev().block_len() * count);
    let mut buf = h.dma_malloc(buflen).expect("failed to allocate buffer");
    buf.fill(fill);

    let s = buf.as_slice();
    assert_eq!(s[0], fill);

    h.write_at(offset, &buf).await
}

pub async fn read_some(
    nexus_name: &str,
    offset: u64,
    num_blocks: u32,
    fill: u8,
) -> Result<(), CoreError> {
    let h = UntypedBdevHandle::open(nexus_name, true, false)?;

    let buflen = u64::from(h.get_bdev().block_len() * num_blocks);
    let mut buf = h.dma_malloc(buflen).expect("failed to allocate buffer");
    let slice = buf.as_mut_slice();

    assert_eq!(slice[0], 0);
    slice[512] = fill;
    assert_eq!(slice[512], fill);

    let len = h.read_at(offset, &mut buf).await?;
    assert_eq!(len, buflen);

    let slice = buf.as_slice();

    for &it in slice.iter().take(512) {
        assert_eq!(it, fill);
    }
    Ok(())
}

pub async fn write_zeroes_some(nexus_name: &str, offset: u64, len: u64) -> Result<(), CoreError> {
    let h = UntypedBdevHandle::open(nexus_name, true, false)?;

    h.write_zeroes_at(offset, len).await?;
    Ok(())
}

pub async fn read_some_safe(nexus_name: &str, offset: u64, fill: u8) -> Result<bool, CoreError> {
    let h = UntypedBdevHandle::open(nexus_name, true, false)?;

    let buflen = u64::from(h.get_bdev().block_len() * 2);
    let mut buf = h.dma_malloc(buflen).expect("failed to allocate buffer");
    let slice = buf.as_mut_slice();

    assert_eq!(slice[0], 0);
    slice[512] = fill;
    assert_eq!(slice[512], fill);

    let len = h.read_at(offset, &mut buf).await?;
    assert_eq!(len, buflen);

    let slice = buf.as_slice();

    for &it in slice.iter().take(512) {
        if it != fill {
            println!("Expected to read {fill}, found {it}");
            return Ok(false);
        }
    }
    Ok(true)
}
