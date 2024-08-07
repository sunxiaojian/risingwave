// Copyright 2024 RisingWave Labs
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use anyhow::anyhow;
use futures_async_stream::try_stream;
use futures_util::stream::StreamExt;
use parquet::arrow::ProjectionMask;
use risingwave_common::array::arrow::IcebergArrowConvert;
use risingwave_common::catalog::Schema;
use risingwave_connector::source::iceberg::parquet_file_reader::create_parquet_stream_builder;

use crate::error::BatchError;
use crate::executor::{DataChunk, Executor};

#[derive(PartialEq, Debug)]
pub enum FileFormat {
    Parquet,
}

/// S3 file scan executor. Currently only support parquet file format.
pub struct S3FileScanExecutor {
    file_format: FileFormat,
    location: String,
    s3_region: String,
    s3_access_key: String,
    s3_secret_key: String,
    batch_size: usize,
    schema: Schema,
    identity: String,
}

impl Executor for S3FileScanExecutor {
    fn schema(&self) -> &risingwave_common::catalog::Schema {
        &self.schema
    }

    fn identity(&self) -> &str {
        &self.identity
    }

    fn execute(self: Box<Self>) -> super::BoxedDataChunkStream {
        self.do_execute().boxed()
    }
}

impl S3FileScanExecutor {
    #![expect(dead_code)]
    pub fn new(
        file_format: FileFormat,
        location: String,
        s3_region: String,
        s3_access_key: String,
        s3_secret_key: String,
        batch_size: usize,
        schema: Schema,
        identity: String,
    ) -> Self {
        Self {
            file_format,
            location,
            s3_region,
            s3_access_key,
            s3_secret_key,
            batch_size,
            schema,
            identity,
        }
    }

    #[try_stream(ok = DataChunk, error = BatchError)]
    async fn do_execute(self: Box<Self>) {
        assert_eq!(self.file_format, FileFormat::Parquet);

        let mut batch_stream_builder = create_parquet_stream_builder(
            self.s3_region.clone(),
            self.s3_access_key.clone(),
            self.s3_secret_key.clone(),
            self.location.clone(),
        )
        .await?;

        let arrow_schema = batch_stream_builder.schema();
        assert_eq!(arrow_schema.fields.len(), self.schema.fields.len());
        for (field, arrow_field) in self.schema.fields.iter().zip(arrow_schema.fields.iter()) {
            assert_eq!(*field.name, *arrow_field.name());
        }

        batch_stream_builder = batch_stream_builder.with_projection(ProjectionMask::all());

        batch_stream_builder = batch_stream_builder.with_batch_size(self.batch_size);

        let record_batch_stream = batch_stream_builder
            .build()
            .map_err(|e| anyhow!(e).context("fail to build arrow stream builder"))?;

        #[for_await]
        for record_batch in record_batch_stream {
            let record_batch = record_batch.map_err(BatchError::Parquet)?;
            let chunk = IcebergArrowConvert.chunk_from_record_batch(&record_batch)?;
            debug_assert_eq!(chunk.data_types(), self.schema.data_types());
            yield chunk;
        }
    }
}
