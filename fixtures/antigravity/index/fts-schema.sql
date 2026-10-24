CREATE VIRTUAL TABLE conversations_fts USING fts5(
			conversation_id UNINDEXED,
			chunk_index UNINDEXED,
			chunk_first_step UNINDEXED,
			chunk_last_step UNINDEXED,
			last_modified_time UNINDEXED,
			step_count UNINDEXED,
			last_step_created_at UNINDEXED,
			archived UNINDEXED,
			user_step_byte_offsets UNINDEXED,
			agent_step_byte_offsets UNINDEXED,
			title,
			workspace,
			project,
			user_body,
			agent_body,
			tokenize = 'unicode61 remove_diacritics 2',
			prefix = '1 2 3'
		);
