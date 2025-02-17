#![allow(deprecated)]

use std::fmt;
use std::io::{self, Read, Write};

use bytes::BytesMut;
use futures::{Poll, Sink,  Stream};
use tokio_codec::{Decoder, Encoder};
use tokio_io::{AsyncRead, AsyncWrite};

use super::framed_read::{framed_read2, framed_read2_with_buffer, FramedRead2};
use super::framed_write::{framed_write2, framed_write2_with_buffer, FramedWrite2};
use std::pin::Pin;
use std::task::Context;

const LW: usize = 1024;
const HW: usize = 8 * 1024;

/// A unified `Stream` and `Sink` interface to an underlying I/O object, using
/// the `Encoder` and `Decoder` traits to encode and decode frames.
///
/// You can create a `Framed` instance by using the `AsyncRead::framed` adapter.
pub struct Framed<T, U> {
    inner: FramedRead2<FramedWrite2<Fuse<T, U>>>,
}

pub struct Fuse<T, U>(pub T, pub U);

impl<T, U> Framed<T, U>
where
    T: AsyncRead + AsyncWrite,
    U: Decoder + Encoder,
{
    /// Provides a `Stream` and `Sink` interface for reading and writing to this
    /// `Io` object, using `Decode` and `Encode` to read and write the raw data.
    ///
    /// Raw I/O objects work with byte sequences, but higher-level code usually
    /// wants to batch these into meaningful chunks, called "frames". This
    /// method layers framing on top of an I/O object, by using the `Codec`
    /// traits to handle encoding and decoding of messages frames. Note that
    /// the incoming and outgoing frame types may be distinct.
    ///
    /// This function returns a *single* object that is both `Stream` and
    /// `Sink`; grouping this into a single object is often useful for layering
    /// things like gzip or TLS, which require both read and write access to the
    /// underlying object.
    ///
    /// If you want to work more directly with the streams and sink, consider
    /// calling `split` on the `Framed` returned by this method, which will
    /// break them into separate objects, allowing them to interact more easily.
    pub fn new(inner: T, codec: U) -> Framed<T, U> {
        Framed {
            inner: framed_read2(framed_write2(Fuse(inner, codec), LW, HW)),
        }
    }

    /// Same as `Framed::new()` with ability to specify write buffer low/high capacity watermarks.
    pub fn new_with_caps(inner: T, codec: U, lw: usize, hw: usize) -> Framed<T, U> {
        debug_assert!((lw < hw) && hw != 0);
        Framed {
            inner: framed_read2(framed_write2(Fuse(inner, codec), lw, hw)),
        }
    }

    /// Force send item
    pub fn force_send(
        &mut self,
        item: <U as Encoder>::Item,
    ) -> Result<(), <U as Encoder>::Error> {
        self.inner.get_mut().force_send(item)
    }
}

impl<T, U> Framed<T, U> {
    /// Provides a `Stream` and `Sink` interface for reading and writing to this
    /// `Io` object, using `Decode` and `Encode` to read and write the raw data.
    ///
    /// Raw I/O objects work with byte sequences, but higher-level code usually
    /// wants to batch these into meaningful chunks, called "frames". This
    /// method layers framing on top of an I/O object, by using the `Codec`
    /// traits to handle encoding and decoding of messages frames. Note that
    /// the incoming and outgoing frame types may be distinct.
    ///
    /// This function returns a *single* object that is both `Stream` and
    /// `Sink`; grouping this into a single object is often useful for layering
    /// things like gzip or TLS, which require both read and write access to the
    /// underlying object.
    ///
    /// This objects takes a stream and a readbuffer and a writebuffer. These
    /// field can be obtained from an existing `Framed` with the
    /// `into_parts` method.
    ///
    /// If you want to work more directly with the streams and sink, consider
    /// calling `split` on the `Framed` returned by this method, which will
    /// break them into separate objects, allowing them to interact more easily.
    pub fn from_parts(parts: FramedParts<T, U>) -> Framed<T, U> {
        Framed {
            inner: framed_read2_with_buffer(
                framed_write2_with_buffer(
                    Fuse(parts.io, parts.codec),
                    parts.write_buf,
                    parts.write_buf_lw,
                    parts.write_buf_hw,
                ),
                parts.read_buf,
            ),
        }
    }

    /// Returns a reference to the underlying codec.
    pub fn get_codec(&self) -> &U {
        &self.inner.get_ref().get_ref().1
    }

    /// Returns a mutable reference to the underlying codec.
    pub fn get_codec_mut(&mut self) -> &mut U {
        &mut self.inner.get_mut().get_mut().1
    }

    /// Returns a reference to the underlying I/O stream wrapped by
    /// `Frame`.
    ///
    /// Note that care should be taken to not tamper with the underlying stream
    /// of data coming in as it may corrupt the stream of frames otherwise
    /// being worked with.
    pub fn get_ref(&self) -> &T {
        &self.inner.get_ref().get_ref().0
    }

    /// Returns a mutable reference to the underlying I/O stream wrapped by
    /// `Frame`.
    ///
    /// Note that care should be taken to not tamper with the underlying stream
    /// of data coming in as it may corrupt the stream of frames otherwise
    /// being worked with.
    pub fn get_mut(&mut self) -> &mut T {
        &mut self.inner.get_mut().get_mut().0
    }

    /// Check if write buffer is empty.
    pub fn is_write_buf_empty(&self) -> bool {
        self.inner.get_ref().is_empty()
    }

    /// Check if write buffer is full.
    pub fn is_write_buf_full(&self) -> bool {
        self.inner.get_ref().is_full()
    }

    /// Consumes the `Frame`, returning its underlying I/O stream.
    ///
    /// Note that care should be taken to not tamper with the underlying stream
    /// of data coming in as it may corrupt the stream of frames otherwise
    /// being worked with.
    pub fn into_inner(self) -> T {
        self.inner.into_inner().into_inner().0
    }

    /// Consume the `Frame`, returning `Frame` with different codec.
    pub fn into_framed<U2>(self, codec: U2) -> Framed<T, U2> {
        let (inner, read_buf) = self.inner.into_parts();
        let (inner, write_buf, lw, hw) = inner.into_parts();

        Framed {
            inner: framed_read2_with_buffer(
                framed_write2_with_buffer(Fuse(inner.0, codec), write_buf, lw, hw),
                read_buf,
            ),
        }
    }

    /// Consume the `Frame`, returning `Frame` with different io.
    pub fn map_io<F, T2>(self, f: F) -> Framed<T2, U>
    where
        F: Fn(T) -> T2,
    {
        let (inner, read_buf) = self.inner.into_parts();
        let (inner, write_buf, lw, hw) = inner.into_parts();

        Framed {
            inner: framed_read2_with_buffer(
                framed_write2_with_buffer(Fuse(f(inner.0), inner.1), write_buf, lw, hw),
                read_buf,
            ),
        }
    }

    /// Consume the `Frame`, returning `Frame` with different codec.
    pub fn map_codec<F, U2>(self, f: F) -> Framed<T, U2>
    where
        F: Fn(U) -> U2,
    {
        let (inner, read_buf) = self.inner.into_parts();
        let (inner, write_buf, lw, hw) = inner.into_parts();

        Framed {
            inner: framed_read2_with_buffer(
                framed_write2_with_buffer(Fuse(inner.0, f(inner.1)), write_buf, lw, hw),
                read_buf,
            ),
        }
    }

    /// Consumes the `Frame`, returning its underlying I/O stream, the buffer
    /// with unprocessed data, and the codec.
    ///
    /// Note that care should be taken to not tamper with the underlying stream
    /// of data coming in as it may corrupt the stream of frames otherwise
    /// being worked with.
    pub fn into_parts(self) -> FramedParts<T, U> {
        let (inner, read_buf) = self.inner.into_parts();
        let (inner, write_buf, write_buf_lw, write_buf_hw) = inner.into_parts();

        FramedParts {
            io: inner.0,
            codec: inner.1,
            read_buf,
            write_buf,
            write_buf_lw,
            write_buf_hw,
            _priv: (),
        }
    }
}


impl<T, U> Stream for Framed<T, U>
where
    T: AsyncRead,
    U: Decoder,
{
    type Item = Result<U::Item,U::Error>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        unsafe { self.map_unchecked_mut(|s| &mut s.inner).poll_next(cx )}
    }


}

impl<T, U> Sink<U::Item> for Framed<T, U>
where
    T: AsyncWrite,
    U: Encoder,
    U::Error: From<io::Error>,
{
    type Error = U::Error;

    fn poll_ready(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        unsafe { self.map_unchecked_mut(|s| s.inner.get_mut()).poll_ready(cx)}
    }

    fn start_send(self: Pin<&mut Self>, item: <U as Encoder>::Item) -> Result<(), Self::Error> {
        unsafe { self.map_unchecked_mut(|s| s.inner.get_mut()).start_send(item)}
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        unsafe { self.map_unchecked_mut(|s| s.inner.get_mut()).poll_flush(cx)}
    }

    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        unsafe { self.map_unchecked_mut(|s| s.inner.get_mut()).poll_close(cx)}
    }
}

impl<T, U> fmt::Debug for Framed<T, U>
where
    T: fmt::Debug,
    U: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("Framed")
            .field("io", &self.inner.get_ref().get_ref().0)
            .field("codec", &self.inner.get_ref().get_ref().1)
            .finish()
    }
}

// ===== impl Fuse =====

impl<T: Read, U> Read for Fuse<T, U> {
    fn read(&mut self, dst: &mut [u8]) -> io::Result<usize> {
        self.0.read(dst)
    }
}


impl<T: AsyncRead, U> AsyncRead for Fuse<T, U> {

    unsafe fn prepare_uninitialized_buffer(&self, buf: &mut [u8]) -> bool {
        self.0.prepare_uninitialized_buffer(buf)
    }

    fn poll_read(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut [u8]) -> Poll<io::Result<usize>> {
        unsafe { self.map_unchecked_mut(|s| &mut s.0).poll_read(cx, buf)}
    }
}

impl<T: Write, U> Write for Fuse<T, U> {
    fn write(&mut self, src: &[u8]) -> io::Result<usize> {
        self.0.write(src)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}


impl<T: AsyncWrite, U> AsyncWrite for Fuse<T, U> {

    fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>> {
        unsafe { self.map_unchecked_mut(|s| &mut s.0).poll_write(cx, buf)}
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        unsafe { self.map_unchecked_mut(|s|  &mut s.0).poll_flush(cx)}
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        unsafe { self.map_unchecked_mut(|s| &mut s.0).poll_shutdown(cx)}
    }
}


impl<T, U: Decoder> Decoder for Fuse<T, U> {
    type Item = U::Item;
    type Error = U::Error;

    fn decode(&mut self, buffer: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        self.1.decode(buffer)
    }

    fn decode_eof(&mut self, buffer: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        self.1.decode_eof(buffer)
    }
}

impl<T, U: Encoder> Encoder for Fuse<T, U> {
    type Item = U::Item;
    type Error = U::Error;

    fn encode(&mut self, item: Self::Item, dst: &mut BytesMut) -> Result<(), Self::Error> {
        self.1.encode(item, dst)
    }
}

/// `FramedParts` contains an export of the data of a Framed transport.
/// It can be used to construct a new `Framed` with a different codec.
/// It contains all current buffers and the inner transport.
#[derive(Debug)]
pub struct FramedParts<T, U> {
    /// The inner transport used to read bytes to and write bytes to
    pub io: T,

    /// The codec
    pub codec: U,

    /// The buffer with read but unprocessed data.
    pub read_buf: BytesMut,

    /// A buffer with unprocessed data which are not written yet.
    pub write_buf: BytesMut,

    /// A buffer low watermark capacity
    pub write_buf_lw: usize,

    /// A buffer high watermark capacity
    pub write_buf_hw: usize,

    /// This private field allows us to add additional fields in the future in a
    /// backwards compatible way.
    _priv: (),
}

impl<T, U> FramedParts<T, U> {
    /// Create a new, default, `FramedParts`
    pub fn new(io: T, codec: U) -> FramedParts<T, U> {
        FramedParts {
            io,
            codec,
            read_buf: BytesMut::new(),
            write_buf: BytesMut::new(),
            write_buf_lw: LW,
            write_buf_hw: HW,
            _priv: (),
        }
    }

    /// Create a new `FramedParts` with read buffer
    pub fn with_read_buf(io: T, codec: U, read_buf: BytesMut) -> FramedParts<T, U> {
        FramedParts {
            io,
            codec,
            read_buf,
            write_buf: BytesMut::new(),
            write_buf_lw: LW,
            write_buf_hw: HW,
            _priv: (),
        }
    }
}
